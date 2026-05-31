pub mod api;
pub mod bedrock;
pub mod openai;
pub mod ollama;
pub mod stdio;
pub mod mock;

use crate::events::RunnerSpec;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub run_id:    String,
    pub stage:     String,
    pub runner:    RunnerSpec,
    pub artifacts: serde_json::Value,
    pub prompt:    Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentResult {
    pub outputs: Vec<(String, String, bool)>,
}

#[async_trait]
pub trait Driver: Send + Sync {
    async fn invoke_agent(&self, invocation: AgentInvocation) -> anyhow::Result<AgentResult>;
}
