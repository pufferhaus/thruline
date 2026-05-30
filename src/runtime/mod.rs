pub mod artifact;
pub mod scheduler;
pub mod state;

use std::collections::HashMap;
use std::path::Path;

use crate::ast::*;
use crate::driver::{AgentInvocation, Driver};
use crate::events::{RunnerSpec, ThrulineEvent};
use crate::runtime::artifact::ArtifactStore;
use crate::runtime::state::{RunState, RunStatus};

pub struct Runtime {
    pub state: RunState,
    pub items: Vec<TlItem>,
}

impl Runtime {
    pub fn new(state: RunState, items: Vec<TlItem>) -> Self {
        Self { state, items }
    }

    fn runners(&self) -> HashMap<String, &RunnerDecl> {
        self.items.iter().filter_map(|i| {
            if let TlItem::Runner(r) = i { Some((r.name.clone(), r)) } else { None }
        }).collect()
    }

    fn stages(&self) -> HashMap<String, &StageDecl> {
        self.items.iter().filter_map(|i| {
            if let TlItem::Stage(s) = i { Some((s.name.clone(), s)) } else { None }
        }).collect()
    }

    fn pipeline(&self, name: &str) -> Option<&PipelineDecl> {
        self.items.iter().find_map(|i| {
            if let TlItem::Pipeline(p) = i { if p.name == name { return Some(p); } }
            None
        })
    }

    /// Evaluate routes from the given source stage. Returns the first matching Route
    /// and its resolved target stage name, or None if no route matches.
    pub fn evaluate_routes<'a>(
        &self,
        from_stage: &str,
        routes: &'a [Route],
        artifacts: &ArtifactStore,
    ) -> Option<(&'a Route, String)> {
        for route in routes {
            let matches = match &route.source {
                RouteSource::Stage(s) => s == from_stage,
                RouteSource::FanIn(s) => s == from_stage,
                RouteSource::Predicate { stage, artifact, op, value } => {
                    if stage != from_stage {
                        continue;
                    }
                    let key = format!("{}.{}", stage, artifact);
                    let actual = artifacts.get_ref(&key).unwrap_or("");
                    match op {
                        CompareOp::Eq => actual == value,
                        CompareOp::Ne => actual != value,
                    }
                }
            };
            if matches {
                return Some((route, route.target.stage.clone()));
            }
        }
        None
    }

    /// Resolve a RunnerDecl to a RunnerSpec, loading file-based system prompts.
    pub fn resolve_runner(
        &self,
        runner: &RunnerDecl,
        base_path: &Path,
    ) -> anyhow::Result<RunnerSpec> {
        let system = match &runner.system {
            PromptSource::Inline(s) => s.clone(),
            PromptSource::File(rel) => {
                let abs = base_path.parent().unwrap_or(Path::new(".")).join(rel);
                std::fs::read_to_string(&abs).map_err(|_| {
                    anyhow::anyhow!("system prompt file not found: {}", abs.display())
                })?
            }
        };
        Ok(RunnerSpec {
            name: runner.name.clone(),
            model: if runner.model.is_empty() { None } else { Some(runner.model.clone()) },
            system,
            tools: runner.tools.clone(),
            temperature: runner.temperature,
            max_tokens: runner.max_tokens,
        })
    }

    /// Build input artifacts JSON for a stage from the store.
    fn stage_input_artifacts(
        &self,
        stage: &StageDecl,
        artifacts: &ArtifactStore,
    ) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for input in &stage.inputs {
            let key = format!("{}.{}", stage.name, input.name);
            if let Some(path) = artifacts.get_file(&key) {
                map.insert(
                    input.name.clone(),
                    serde_json::Value::String(path.to_string_lossy().into_owned()),
                );
            } else if let Some(val) = artifacts.get_ref(&key) {
                map.insert(
                    input.name.clone(),
                    serde_json::Value::String(val.to_string()),
                );
            }
        }
        serde_json::Value::Object(map)
    }

    /// Advance the pipeline one step: invoke the current stage via the driver.
    /// For the stdio driver this emits stage_invoke, saves state, and returns.
    pub async fn advance(&mut self, driver: &dyn Driver) -> anyhow::Result<bool> {
        let pipeline_name = self.state.pipeline.clone();
        let pipeline = self
            .pipeline(&pipeline_name)
            .ok_or_else(|| anyhow::anyhow!("pipeline '{}' not found", pipeline_name))?;

        let current_stage_name = match &self.state.status {
            RunStatus::Running => pipeline.start.clone(),
            RunStatus::AwaitingResume { stage } => stage.clone(),
            RunStatus::Done | RunStatus::Failed(_) | RunStatus::ParallelAwait { .. } => {
                return Ok(false)
            }
        };

        let stages = self.stages();
        let stage = stages
            .get(&current_stage_name)
            .ok_or_else(|| anyhow::anyhow!("stage '{}' not found", current_stage_name))?;

        let runners = self.runners();
        let runner_decl = runners
            .get(&stage.agent)
            .ok_or_else(|| anyhow::anyhow!("runner '{}' not found", stage.agent))?;

        let tl_path = self.state.line_file.clone();
        let runner_spec = self.resolve_runner(runner_decl, &tl_path)?;
        let input_artifacts = self.stage_input_artifacts(stage, &self.state.artifacts);

        let prompt = match &stage.prompt {
            Some(PromptSource::Inline(s)) => Some(s.clone()),
            Some(PromptSource::File(p)) => {
                let abs = tl_path.parent().unwrap_or(Path::new(".")).join(p);
                Some(std::fs::read_to_string(&abs)?)
            }
            None => None,
        };

        ThrulineEvent::StageInvoke {
            run_id: self.state.run_id.clone(),
            ts: chrono::Utc::now(),
            stage: current_stage_name.clone(),
            runner: runner_spec.clone(),
            artifacts: input_artifacts.clone(),
            prompt: prompt.clone(),
        }
        .emit();

        self.state.status = RunStatus::AwaitingResume {
            stage: current_stage_name.clone(),
        };
        self.state.save()?;

        let invocation = AgentInvocation {
            run_id: self.state.run_id.clone(),
            stage: current_stage_name.clone(),
            runner: runner_spec,
            artifacts: input_artifacts,
            prompt,
        };
        driver.invoke_agent(invocation).await?;

        Ok(true)
    }

    /// Feed completed stage output back into the state machine.
    /// Updates artifact store, evaluates routes, saves state.
    pub fn resume_stage(
        &mut self,
        stage_name: &str,
        outputs: Vec<(String, String, bool)>,
    ) -> anyhow::Result<()> {
        for (name, value, is_file) in outputs {
            let key = format!("{}.{}", stage_name, name);
            if is_file {
                self.state.artifacts.set_file(&key, &value);
            } else {
                self.state.artifacts.set_ref(&key, &value);
            }
        }

        self.state.history.push(stage_name.to_string());

        let pipeline_name = self.state.pipeline.clone();
        let pipeline = self
            .pipeline(&pipeline_name)
            .ok_or_else(|| anyhow::anyhow!("pipeline '{}' not found", pipeline_name))?;

        let routes: Vec<Route> = pipeline.routes.clone();
        let artifacts_snapshot = self.state.artifacts.clone();

        if let Some((route, next_stage)) =
            self.evaluate_routes(stage_name, &routes, &artifacts_snapshot)
        {
            // TODO: if route.parallel is true, use Scheduler for fan-out/fan-in
            // instead of single AwaitingResume. ParallelStart/ParallelSlotOpen/
            // ParallelDone events and RunStatus::ParallelAwait are defined but
            // not yet wired here.
            let predicate_desc = format!("{:?}", route.source);
            ThrulineEvent::RouteTaken {
                run_id: self.state.run_id.clone(),
                ts: chrono::Utc::now(),
                from: stage_name.to_string(),
                to: next_stage.clone(),
                predicate: predicate_desc,
            }
            .emit();

            self.state.status = RunStatus::AwaitingResume { stage: next_stage };
        } else {
            let outputs: Vec<String> = self.state.artifacts.file_keys().cloned().collect();
            ThrulineEvent::PipelineDone {
                run_id: self.state.run_id.clone(),
                ts: chrono::Utc::now(),
                outputs,
            }
            .emit();
            self.state.status = RunStatus::Done;
        }

        self.state.save()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;
    use crate::runtime::artifact::ArtifactStore;
    use crate::runtime::state::RunState;

    fn mk_runner(name: &str) -> TlItem {
        TlItem::Runner(RunnerDecl {
            name: name.to_string(),
            model: "claude-sonnet-4-6".to_string(),
            system: PromptSource::Inline("system prompt".to_string()),
            tools: vec![],
            temperature: None,
            max_tokens: None,
        })
    }

    fn mk_stage(name: &str, agent: &str, outputs: &[(&str, ArtifactKind)]) -> TlItem {
        TlItem::Stage(StageDecl {
            name: name.to_string(),
            inputs: vec![],
            outputs: outputs
                .iter()
                .map(|(n, k)| ArtifactDecl {
                    name: n.to_string(),
                    optional: false,
                    kind: k.clone(),
                    seed_path: None,
                })
                .collect(),
            agent: agent.to_string(),
            prompt: None,
            format: None,
        })
    }

    fn mk_runtime() -> Runtime {
        let state = RunState::new("r1".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            mk_stage("a", "runner", &[("verdict", ArtifactKind::Ref)]),
            mk_stage("b", "runner", &[]),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(),
                start: "a".into(),
                routes: vec![
                    Route {
                        source: RouteSource::Predicate {
                            stage: "a".into(),
                            artifact: "verdict".into(),
                            op: CompareOp::Eq,
                            value: "ok".into(),
                        },
                        target: RouteTarget {
                            stage: "b".into(),
                            parallel_spec: None,
                        },
                        parallel: false,
                    },
                    Route {
                        source: RouteSource::Predicate {
                            stage: "a".into(),
                            artifact: "verdict".into(),
                            op: CompareOp::Ne,
                            value: "ok".into(),
                        },
                        target: RouteTarget {
                            stage: "a".into(),
                            parallel_spec: None,
                        },
                        parallel: false,
                    },
                ],
            }),
        ];
        Runtime::new(state, items)
    }

    #[test]
    fn test_evaluate_routes_eq_match() {
        let rt = mk_runtime();
        let pipeline = rt.pipeline("p").unwrap();
        let mut store = ArtifactStore::new();
        store.set_ref("a.verdict", "ok");
        let result = rt.evaluate_routes("a", &pipeline.routes, &store);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, "b");
    }

    #[test]
    fn test_evaluate_routes_ne_retry() {
        let rt = mk_runtime();
        let pipeline = rt.pipeline("p").unwrap();
        let mut store = ArtifactStore::new();
        store.set_ref("a.verdict", "rejected");
        let result = rt.evaluate_routes("a", &pipeline.routes, &store);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, "a"); // retry
    }

    #[test]
    fn test_evaluate_routes_no_match() {
        let rt = mk_runtime();
        let pipeline = rt.pipeline("p").unwrap();
        let store = ArtifactStore::new(); // no verdict set
        // Both predicates check "a.verdict" — with empty store it returns "" which != "ok"
        // so Ne predicate matches → routes to "a" (retry)
        let result = rt.evaluate_routes("a", &pipeline.routes, &store);
        assert!(result.is_some()); // Ne always matches when value is ""
    }

    #[test]
    fn test_evaluate_unconditional_route() {
        let state = RunState::new("r".into(), "p2".into(), "/tmp/t.line".into());
        let items = vec![
            mk_runner("r"),
            mk_stage("x", "r", &[]),
            mk_stage("y", "r", &[]),
            TlItem::Pipeline(PipelineDecl {
                name: "p2".into(),
                start: "x".into(),
                routes: vec![Route {
                    source: RouteSource::Stage("x".into()),
                    target: RouteTarget { stage: "y".into(), parallel_spec: None },
                    parallel: false,
                }],
            }),
        ];
        let rt = Runtime::new(state, items);
        let pipeline = rt.pipeline("p2").unwrap();
        let store = ArtifactStore::new();
        let result = rt.evaluate_routes("x", &pipeline.routes, &store);
        assert_eq!(result.unwrap().1, "y");
    }

    #[test]
    fn test_resolve_runner_inline() {
        let rt = mk_runtime();
        let runners = rt.runners();
        let runner_decl = runners.get("runner").unwrap();
        let spec = rt.resolve_runner(runner_decl, Path::new("/tmp/test.line")).unwrap();
        assert_eq!(spec.model, Some("claude-sonnet-4-6".to_string()));
        assert_eq!(spec.system, "system prompt");
    }

    #[test]
    fn test_resume_stage_updates_artifacts_and_advances() {
        let mut rt = mk_runtime();
        rt.state.status = crate::runtime::state::RunStatus::AwaitingResume { stage: "a".into() };
        // Feed verdict=ok back
        rt.resume_stage("a", vec![("verdict".to_string(), "ok".to_string(), false)]).unwrap();
        // Should advance to stage "b"
        assert!(matches!(&rt.state.status, RunStatus::AwaitingResume { stage } if stage == "b"));
        // Artifact should be stored
        assert_eq!(rt.state.artifacts.get_ref("a.verdict"), Some("ok"));
    }
}
