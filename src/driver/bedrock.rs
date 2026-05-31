use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::driver::{AgentInvocation, AgentResult, Driver};

pub struct BedrockDriver {
    pub region: String,
    pub default_model: Option<String>,
}

impl BedrockDriver {
    pub fn from_env(default_model: Option<String>) -> Self {
        let region = std::env::var("AWS_REGION")
            .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_string());
        Self { region, default_model }
    }
}

#[derive(Serialize)]
struct MessagesRequest {
    anthropic_version: String,
    max_tokens: u32,
    messages: Vec<BedrockMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
}

#[derive(Serialize)]
struct BedrockMessage {
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
impl Driver for BedrockDriver {
    async fn invoke_agent(&self, invocation: AgentInvocation) -> anyhow::Result<AgentResult> {
        let model_id = invocation.runner.model.clone()
            .or_else(|| self.default_model.clone())
            .or_else(|| std::env::var("BEDROCK_DEFAULT_MODEL").ok())
            .ok_or_else(|| anyhow::anyhow!(
                "runner '{}' has no model — declare one or set BEDROCK_DEFAULT_MODEL \
                 (e.g. anthropic.claude-3-5-sonnet-20241022-v2:0)",
                invocation.runner.name
            ))?;

        // Build user message
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

        let req = MessagesRequest {
            anthropic_version: "bedrock-2023-05-31".to_string(),
            max_tokens: invocation.runner.max_tokens.unwrap_or(8096),
            system: invocation.runner.system.clone(),
            messages: vec![BedrockMessage {
                role: "user".into(),
                content: user_parts.join("\n\n"),
            }],
            temperature: invocation.runner.temperature,
        };

        let body_bytes = serde_json::to_vec(&req)?;

        // Build AWS client
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_sdk_bedrockruntime::config::Region::new(self.region.clone()))
            .load()
            .await;
        let client = aws_sdk_bedrockruntime::Client::new(&config);

        let response = client
            .invoke_model()
            .model_id(&model_id)
            .content_type("application/json")
            .accept("application/json")
            .body(aws_sdk_bedrockruntime::primitives::Blob::new(body_bytes))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("bedrock invoke_model failed: {}", e))?;

        let resp_bytes = response.body.into_inner();
        let resp: MessagesResponse = serde_json::from_slice(&resp_bytes)
            .map_err(|e| anyhow::anyhow!("bedrock response parse error: {}", e))?;

        let text = resp.content.into_iter()
            .filter(|b| b.kind == "text")
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("\n");

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
    fn test_parse_bedrock_output() {
        let text = r#"{"verdict": "approved", "notes": "Clean implementation"}"#;
        let outputs = parse_agent_output(text);
        assert!(outputs.iter().any(|(k, v, f)| k == "verdict" && v == "approved" && !f));
        assert!(outputs.iter().any(|(k, v, f)| k == "notes" && v == "Clean implementation" && !f));
    }

    #[test]
    fn test_parse_bedrock_markdown_block() {
        let text = "Here is the output:\n```json\n{\"verdict\": \"rejected\"}\n```";
        let outputs = parse_agent_output(text);
        assert!(outputs.iter().any(|(k, v, _)| k == "verdict" && v == "rejected"));
    }

    #[test]
    fn test_parse_bedrock_file_artifact() {
        let text = r#"{"spec": "file:///tmp/spec.md"}"#;
        let outputs = parse_agent_output(text);
        assert!(outputs.iter().any(|(k, v, f)| k == "spec" && v == "/tmp/spec.md" && *f));
    }
}
