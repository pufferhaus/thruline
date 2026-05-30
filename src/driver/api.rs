use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use crate::driver::{AgentInvocation, AgentResult, Driver};

pub struct ApiDriver {
    pub api_key: String,
}

impl ApiDriver {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY environment variable not set"))?;
        Ok(Self::new(key))
    }
}

#[derive(Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[async_trait]
impl Driver for ApiDriver {
    async fn invoke_agent(&self, invocation: AgentInvocation) -> anyhow::Result<AgentResult> {
        let user_content = build_user_message(&invocation);

        let req = MessagesRequest {
            model: invocation.runner.model.clone().unwrap_or_else(|| "claude-sonnet-4-6".to_string()),
            max_tokens: invocation.runner.max_tokens.unwrap_or(8096),
            system: invocation.runner.system.clone(),  // None = harness default, omitted from API request
            messages: vec![ApiMessage {
                role: "user".into(),
                content: user_content,
            }],
            temperature: invocation.runner.temperature,
        };

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("x-api-key", HeaderValue::from_str(&self.api_key)?);
        headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));

        let client = reqwest::Client::new();
        let resp = client
            .post("https://api.anthropic.com/v1/messages")
            .headers(headers)
            .json(&req)
            .send()
            .await?
            .error_for_status()?;

        let body: MessagesResponse = resp.json().await?;
        let text = body
            .content
            .into_iter()
            .filter(|b| b.kind == "text")
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("\n");

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
