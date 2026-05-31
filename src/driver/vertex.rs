use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use crate::driver::{AgentInvocation, AgentResult, Driver};

pub struct VertexDriver {
    pub project: String,
    pub region: String,
    pub access_token: String,
    pub default_model: Option<String>,
}

impl VertexDriver {
    pub fn from_env(default_model: Option<String>) -> anyhow::Result<Self> {
        let access_token = std::env::var("VERTEX_ACCESS_TOKEN")
            .map_err(|_| anyhow::anyhow!(
                "VERTEX_ACCESS_TOKEN not set — run: gcloud auth print-access-token"
            ))?;
        let project = std::env::var("VERTEX_PROJECT")
            .map_err(|_| anyhow::anyhow!("VERTEX_PROJECT not set — set to your GCP project ID"))?;
        let region = std::env::var("VERTEX_REGION")
            .unwrap_or_else(|_| "us-east5".to_string());
        Ok(Self { project, region, access_token, default_model })
    }
}

#[derive(Serialize)]
struct MessagesRequest {
    anthropic_version: String,
    max_tokens: u32,
    messages: Vec<VertexMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
}

#[derive(Serialize)]
struct VertexMessage {
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
impl Driver for VertexDriver {
    async fn invoke_agent(&self, invocation: AgentInvocation) -> anyhow::Result<AgentResult> {
        let model_id = invocation.runner.model.clone()
            .or_else(|| self.default_model.clone())
            .or_else(|| std::env::var("VERTEX_DEFAULT_MODEL").ok())
            .ok_or_else(|| anyhow::anyhow!(
                "runner '{}' has no model — declare one or set VERTEX_DEFAULT_MODEL \
                 (e.g. claude-3-5-sonnet-v2@20241022)",
                invocation.runner.name
            ))?;

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
            anthropic_version: "vertex-2023-10-16".to_string(),
            max_tokens: invocation.runner.max_tokens.unwrap_or(8096),
            system: invocation.runner.system.clone(),
            messages: vec![VertexMessage {
                role: "user".into(),
                content: user_parts.join("\n\n"),
            }],
            temperature: invocation.runner.temperature,
        };

        let url = format!(
            "https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/anthropic/models/{model}:rawPredict",
            region = self.region,
            project = self.project,
            model = model_id,
        );

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.access_token))?,
        );

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .headers(headers)
            .json(&req)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("vertex request failed: {}", e))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("vertex error: {} — check VERTEX_ACCESS_TOKEN is fresh", e))?;

        let body: MessagesResponse = resp.json().await
            .map_err(|e| anyhow::anyhow!("vertex response parse error: {}", e))?;

        let text = body.content.into_iter()
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
    fn test_url_construction() {
        let driver = VertexDriver {
            project: "my-project".into(),
            region: "us-east5".into(),
            access_token: "token".into(),
            default_model: None,
        };
        // Just verify the driver constructs without panic
        assert_eq!(driver.region, "us-east5");
        assert_eq!(driver.project, "my-project");
    }

    #[test]
    fn test_parse_vertex_output() {
        let text = r#"{"verdict": "approved", "notes": "Clean"}"#;
        let out = parse_agent_output(text);
        assert!(out.iter().any(|(k, v, _)| k == "verdict" && v == "approved"));
    }
}
