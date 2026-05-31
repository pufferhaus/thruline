use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use crate::driver::{AgentInvocation, AgentResult, Driver};

pub struct OllamaDriver {
    pub base_url: String,
    pub default_model: Option<String>,
}

impl OllamaDriver {
    pub fn new(base_url: String, default_model: Option<String>) -> Self {
        Self { base_url, default_model }
    }

    pub fn from_env(default_model: Option<String>) -> Self {
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        Self::new(base_url, default_model)
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
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
impl Driver for OllamaDriver {
    async fn invoke_agent(&self, invocation: AgentInvocation) -> anyhow::Result<AgentResult> {
        let model = invocation.runner.model.clone()
            .or_else(|| self.default_model.clone())
            .ok_or_else(|| anyhow::anyhow!(
                "runner '{}' has no model — declare one in the runner block or set OLLAMA_DEFAULT_MODEL",
                invocation.runner.name
            ))?;

        let mut messages = Vec::new();

        if let Some(system) = &invocation.runner.system {
            messages.push(ChatMessage { role: "system".into(), content: system.clone() });
        }

        // Build user message from artifacts and prompt
        let mut user_parts = Vec::new();
        if let Some(prompt) = &invocation.prompt {
            user_parts.push(format!("## Task\n{}", prompt));
        }
        let artifacts_str = serde_json::to_string_pretty(&invocation.artifacts).unwrap_or_default();
        if artifacts_str != "null" && artifacts_str != "{}" {
            user_parts.push(format!("## Input Artifacts\n```json\n{}\n```", artifacts_str));
        }
        user_parts.push(
            "## Output\nRespond with a JSON object mapping output artifact names to their values. \
             For file artifacts, write the file and return the absolute path prefixed with `file://`."
                .to_string(),
        );
        messages.push(ChatMessage { role: "user".into(), content: user_parts.join("\n\n") });

        let options = if invocation.runner.max_tokens.is_some() {
            Some(OllamaOptions { num_predict: invocation.runner.max_tokens })
        } else {
            None
        };

        let req = ChatRequest {
            model,
            messages,
            stream: false,
            temperature: invocation.runner.temperature,
            options,
        };

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .headers(headers)
            .json(&req)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("ollama request failed: {} — is ollama running at {}?", e, self.base_url))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("ollama error: {}", e))?;

        let body: ChatResponse = resp.json().await
            .map_err(|e| anyhow::anyhow!("ollama response parse error: {}", e))?;

        let text = body.choices.into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();

        let outputs = parse_agent_output(&text);
        Ok(AgentResult { outputs })
    }
}

fn extract_json_object(text: &str) -> Option<&str> {
    if let Some(start) = text.find("```json\n") {
        let content_start = start + 8;
        if let Some(end_offset) = text[content_start..].find("```") {
            return Some(text[content_start..content_start + end_offset].trim());
        }
    }
    let start = text.find('{')?;
    let end = text.rfind('}').map(|i| i + 1)?;
    if end > start { Some(&text[start..end]) } else { None }
}

fn parse_agent_output(text: &str) -> Vec<(String, String, bool)> {
    let json_str = extract_json_object(text).unwrap_or(text);
    let Ok(obj) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json_str)
    else { return vec![]; };
    obj.into_iter().map(|(k, v)| {
        let val = match &v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let is_file = val.starts_with("file://");
        let clean = if is_file { val[7..].to_string() } else { val };
        (k, clean, is_file)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_agent_output_refs() {
        let text = r#"{"verdict": "approved", "score": "87"}"#;
        let outputs = parse_agent_output(text);
        assert!(outputs.iter().any(|(k, v, f)| k == "verdict" && v == "approved" && !f));
        assert!(outputs.iter().any(|(k, v, f)| k == "score" && v == "87" && !f));
    }

    #[test]
    fn test_parse_agent_output_file() {
        let text = r#"{"spec": "file:///tmp/spec.md"}"#;
        let outputs = parse_agent_output(text);
        assert!(outputs.iter().any(|(k, v, f)| k == "spec" && v == "/tmp/spec.md" && *f));
    }

    #[test]
    fn test_parse_agent_output_invalid_returns_empty() {
        let outputs = parse_agent_output("not json at all");
        assert!(outputs.is_empty());
    }

    #[test]
    fn test_extract_json_from_markdown_block() {
        let text = "Some explanation\n```json\n{\"key\": \"val\"}\n```\nMore text";
        let extracted = extract_json_object(text).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(extracted).unwrap();
        assert_eq!(parsed["key"], "val");
    }

    #[test]
    fn test_extract_json_bare_object() {
        let text = "Here is your answer: {\"result\": \"ok\"}";
        let extracted = extract_json_object(text).unwrap();
        assert!(extracted.contains("result"));
    }
}
