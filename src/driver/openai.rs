use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use crate::driver::{AgentInvocation, AgentResult, Driver};

pub struct OpenAiDriver {
    pub api_key: String,
    pub base_url: String,
    pub default_model: Option<String>,
}

impl OpenAiDriver {
    pub fn new(api_key: String, default_model: Option<String>) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com".to_string(),
            default_model,
        }
    }

    pub fn from_env(default_model: Option<String>) -> anyhow::Result<Self> {
        let key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| anyhow::anyhow!("OPENAI_API_KEY environment variable not set"))?;
        Ok(Self::new(key, default_model))
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[async_trait]
impl Driver for OpenAiDriver {
    async fn invoke_agent(&self, invocation: AgentInvocation) -> anyhow::Result<AgentResult> {
        let user_content = build_user_message(&invocation);

        let model = invocation.runner.model.clone()
            .or_else(|| self.default_model.clone())
            .or_else(|| std::env::var("OPENAI_DEFAULT_MODEL").ok())
            .ok_or_else(|| anyhow::anyhow!(
                "runner '{}' has no model — declare one in the runner, set config {{ model: ... }}, or set OPENAI_DEFAULT_MODEL",
                invocation.runner.name
            ))?;

        let mut messages = Vec::new();

        if let Some(system) = &invocation.runner.system {
            messages.push(ChatMessage { role: "system".into(), content: system.clone() });
        }

        messages.push(ChatMessage { role: "user".into(), content: user_content });

        let req = ChatRequest {
            model,
            messages,
            temperature: invocation.runner.temperature,
            max_completion_tokens: invocation.runner.max_tokens,
        };

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))?,
        );

        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .headers(headers)
            .json(&req)
            .send()
            .await?
            .error_for_status()?;

        let body: ChatResponse = resp.json().await?;
        let text = body
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();

        let outputs = parse_agent_output(&text);
        Ok(AgentResult { outputs })
    }
}

fn build_user_message(inv: &AgentInvocation) -> String {
    let mut parts = Vec::new();
    if let Some(prompt) = &inv.prompt {
        parts.push(format!("## Task\n{}", prompt));
    }
    let artifacts_str = serde_json::to_string_pretty(&inv.artifacts).unwrap_or_default();
    if artifacts_str != "null" && artifacts_str != "{}" {
        parts.push(format!("## Input Artifacts\n```json\n{}\n```", artifacts_str));
    }
    parts.push(
        "## Output\nRespond with a JSON object mapping output artifact names to their values. \
         For file artifacts, write the file and provide the absolute path prefixed with `file://`."
            .to_string(),
    );
    parts.join("\n\n")
}

/// Parse the agent's response text into artifact outputs.
/// Expects a JSON object; file artifacts have values prefixed with "file://".
fn parse_agent_output(text: &str) -> Vec<(String, String, bool)> {
    let json_str = extract_json_object(text).unwrap_or(text);
    let Ok(obj) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json_str)
    else {
        return vec![];
    };
    obj.into_iter()
        .map(|(k, v)| {
            let val = match &v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let is_file = val.starts_with("file://");
            let clean = if is_file { val[7..].to_string() } else { val };
            (k, clean, is_file)
        })
        .collect()
}

/// Extract the first JSON object from a text that may contain markdown fences.
fn extract_json_object(text: &str) -> Option<&str> {
    // Try ```json ... ``` block first
    if let Some(start) = text.find("```json\n") {
        let content_start = start + 8;
        if let Some(end_offset) = text[content_start..].find("```") {
            return Some(text[content_start..content_start + end_offset].trim());
        }
    }
    // Fall back to finding outermost { ... }
    let start = text.find('{')?;
    let end = text.rfind('}').map(|i| i + 1)?;
    if end > start { Some(&text[start..end]) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_openai_output() {
        let text = r#"{"verdict": "approved", "notes": "Clean implementation"}"#;
        let outputs = parse_agent_output(text);
        assert!(outputs.iter().any(|(k, v, f)| k == "verdict" && v == "approved" && !f));
        assert!(outputs.iter().any(|(k, v, f)| k == "notes" && v == "Clean implementation" && !f));
    }

    #[test]
    fn test_parse_openai_markdown_block() {
        let text = "Here is the output:\n```json\n{\"verdict\": \"rejected\"}\n```";
        let outputs = parse_agent_output(text);
        assert!(outputs.iter().any(|(k, v, _)| k == "verdict" && v == "rejected"));
    }

    #[test]
    fn test_parse_openai_file_artifact() {
        let text = r#"{"spec": "file:///tmp/spec.md"}"#;
        let outputs = parse_agent_output(text);
        assert!(outputs.iter().any(|(k, v, f)| k == "spec" && v == "/tmp/spec.md" && *f));
    }

    #[test]
    fn test_build_user_message_with_prompt_and_artifacts() {
        let inv = AgentInvocation {
            run_id: "r".to_string(),
            stage: "s".to_string(),
            runner: crate::events::RunnerSpec {
                name: "r".to_string(), model: None, system: Some("s".to_string()),
                tools: vec![], temperature: None, max_tokens: None,
            },
            artifacts: serde_json::json!({"brief": "/tmp/brief.md"}),
            prompt: Some("Write the spec.".to_string()),
        };
        let msg = build_user_message(&inv);
        assert!(msg.contains("## Task"));
        assert!(msg.contains("Write the spec."));
        assert!(msg.contains("## Input Artifacts"));
        assert!(msg.contains("## Output"));
    }
}
