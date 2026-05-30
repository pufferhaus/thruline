use async_trait::async_trait;
use crate::driver::{AgentInvocation, AgentResult, Driver};

/// The stdio driver enables harness-controlled pipeline execution.
/// When invoke_agent is called, the stage_invoke event has already been emitted
/// and state has been checkpointed. This driver returns immediately with no outputs —
/// the caller exits after, and the harness resumes via `thruline resume`.
pub struct StdioDriver;

#[async_trait]
impl Driver for StdioDriver {
    async fn invoke_agent(&self, _invocation: AgentInvocation) -> anyhow::Result<AgentResult> {
        Ok(AgentResult { outputs: vec![] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RunnerSpec;

    #[tokio::test]
    async fn test_stdio_driver_returns_empty_result() {
        let driver = StdioDriver;
        let invocation = AgentInvocation {
            run_id: "test".to_string(),
            stage: "interview".to_string(),
            runner: RunnerSpec {
                name: "r".to_string(),
                model: None,
                system: None,
                tools: vec![],
                temperature: None,
                max_tokens: None,
            },
            artifacts: serde_json::json!({}),
            prompt: None,
        };
        let result = driver.invoke_agent(invocation).await.unwrap();
        assert!(result.outputs.is_empty());
    }
}
