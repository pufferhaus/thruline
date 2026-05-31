use async_trait::async_trait;
use std::collections::HashMap;
use crate::driver::{AgentInvocation, AgentResult, Driver};

/// Mock driver — returns pre-scripted outputs from a JSON fixture file.
///
/// Fixture format (JSON):
/// ```json
/// {
///   "classify": { "sentiment": "positive", "score": "9" },
///   "review":   { "verdict": "approved", "notes": "LGTM" },
///   "review.quality": { "verdict": "approved", "notes": "Clean" }
/// }
/// ```
///
/// Key is `stage_name` for regular stages, `stage_name.run_name` for run blocks.
/// File artifacts: prefix value with `file://`, e.g. `"spec": "file:///tmp/spec.md"`.
pub struct MockDriver {
    responses: HashMap<String, HashMap<String, String>>,
}

impl MockDriver {
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|_| anyhow::anyhow!("mock file not found: {}", path.display()))?;
        let responses: HashMap<String, HashMap<String, String>> =
            serde_json::from_str(&content)
                .map_err(|e| anyhow::anyhow!("invalid mock file {}: {}", path.display(), e))?;
        Ok(Self { responses })
    }
}

#[async_trait]
impl Driver for MockDriver {
    async fn invoke_agent(&self, invocation: AgentInvocation) -> anyhow::Result<AgentResult> {
        // Try stage.run first (won't exist for regular stages, only run blocks)
        // but we don't have run_name available here at the AgentInvocation level.
        // The key will just be the stage name.
        let key = invocation.stage.clone();

        let stage_map = self.responses.get(&key)
            .ok_or_else(|| anyhow::anyhow!(
                "mock: no response defined for stage '{}' — add it to the mock file",
                key
            ))?;

        let outputs = stage_map.iter()
            .map(|(k, v)| {
                let is_file = v.starts_with("file://");
                let val = if is_file { v[7..].to_string() } else { v.clone() };
                (k.clone(), val, is_file)
            })
            .collect();

        Ok(AgentResult { outputs })
    }
}
