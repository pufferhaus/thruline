pub mod artifact;
pub mod scheduler;
pub mod state;

use std::collections::HashMap;
use std::path::Path;

use crate::ast::*;
use crate::driver::{AgentInvocation, AgentResult, Driver};
use crate::events::{OutputDecl, RunnerSpec, ThrulineEvent};
use crate::runtime::artifact::ArtifactStore;
use crate::runtime::state::{RunState, RunStatus};

pub enum AdvanceOutcome {
    Invoked { stage: String, result: AgentResult },
    RunsDispatched { stage: String },
    Idle,
}

pub struct Runtime {
    pub state: RunState,
    pub items: Vec<TlItem>,
}

fn format_route_source(source: &RouteSource) -> String {
    match source {
        RouteSource::Stage(s) => format!("Stage({})", s),
        RouteSource::FanIn(s) => format!("FanIn({})", s),
        RouteSource::Predicate { stage, artifact, op, value } => {
            let op_str = match op { CompareOp::Eq => "==", CompareOp::Ne => "!=" };
            format!("{}.{} {} \"{}\"", stage, artifact, op_str, value)
        }
    }
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
            None => None,
            Some(PromptSource::Inline(s)) => Some(s.clone()),
            Some(PromptSource::File(rel)) => {
                let abs = base_path.parent().unwrap_or(Path::new(".")).join(rel);
                Some(std::fs::read_to_string(&abs).map_err(|_| {
                    anyhow::anyhow!("system prompt file not found: {}", abs.display())
                })?)
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
    ///
    /// Resolution order for each declared input:
    ///
    /// If `source` is explicit (e.g. `classify.language as value` or `input.code as value`):
    ///   - Look up that exact key directly. No fallback.
    ///   - `input.x` resolves to the pipeline input namespace (seeded via --input).
    ///
    /// If unqualified (e.g. `code as value`):
    ///   1. `stage.artifact`   — the stage's own prior output (e.g. retry wrote it)
    ///   2. `<prior>.artifact` — scan completed stages newest-first; first match wins
    ///   3. `input.artifact`   — pipeline input (true default: used only if no stage produced it)
    ///
    /// Pipeline inputs are seeds, not overrides. A stage that revises `code` and writes
    /// `revise.code` will have that version propagate forward, shadowing the original
    /// pipeline input for subsequent stages that declare `in: code`.
    fn stage_input_artifacts(
        &self,
        stage: &StageDecl,
        artifacts: &ArtifactStore,
    ) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for input in &stage.inputs {
            let value = if let Some(src) = &input.source {
                // Explicit source: look up key directly (works for both stage names and "input")
                let key = format!("{}.{}", src, input.name);
                if let Some(path) = artifacts.get_file(&key) {
                    Some(serde_json::Value::String(path.to_string_lossy().into_owned()))
                } else {
                    artifacts.get_ref(&key).map(|v| serde_json::Value::String(v.to_string()))
                }
            } else {
                let stage_key = format!("{}.{}", stage.name, input.name);
                let input_key = format!("input.{}", input.name);

                if let Some(path) = artifacts.get_file(&stage_key) {
                    Some(serde_json::Value::String(path.to_string_lossy().into_owned()))
                } else if let Some(val) = artifacts.get_ref(&stage_key) {
                    Some(serde_json::Value::String(val.to_string()))
                } else {
                    // Scan history newest-first — history beats pipeline inputs
                    let mut found = None;
                    for prior in self.state.history.iter().rev() {
                        let key = format!("{}.{}", prior, input.name);
                        if let Some(path) = artifacts.get_file(&key) {
                            found = Some(serde_json::Value::String(path.to_string_lossy().into_owned()));
                            break;
                        } else if let Some(val) = artifacts.get_ref(&key) {
                            found = Some(serde_json::Value::String(val.to_string()));
                            break;
                        }
                    }
                    // Pipeline input is the last resort default
                    found
                        .or_else(|| {
                            if let Some(path) = artifacts.get_file(&input_key) {
                                Some(serde_json::Value::String(path.to_string_lossy().into_owned()))
                            } else {
                                artifacts.get_ref(&input_key).map(|v| serde_json::Value::String(v.to_string()))
                            }
                        })
                        .or_else(|| {
                            input.seed_path.as_ref().map(|seed| {
                                let abs = self.state.line_file
                                    .parent()
                                    .unwrap_or(Path::new("."))
                                    .join(seed);
                                serde_json::Value::String(abs.to_string_lossy().into_owned())
                            })
                        })
                }
            };

            if let Some(v) = value {
                map.insert(input.name.clone(), v);
            }
        }
        serde_json::Value::Object(map)
    }

    /// Validate that all required pipeline inputs are present before first advance.
    pub fn check_pipeline_inputs(&self) -> anyhow::Result<()> {
        let pipeline = match self.items.iter().find_map(|i| {
            if let TlItem::Pipeline(p) = i { if p.name == self.state.pipeline { return Some(p); } }
            None
        }) {
            Some(p) => p,
            None => return Ok(()),
        };
        for input in &pipeline.inputs {
            if input.optional { continue; }
            let key = format!("input.{}", input.name);
            let present = self.state.artifacts.get_file(&key).is_some()
                || self.state.artifacts.get_ref(&key).is_some();
            if !present {
                anyhow::bail!(
                    "missing required pipeline input '{}' — provide with --input {}=<value>",
                    input.name, input.name
                );
            }
        }
        Ok(())
    }

    /// Build the list of AgentInvocations for a stage in ParallelAwait status.
    /// Used by the API driver to invoke run blocks sequentially.
    pub fn pending_run_invocations(&self) -> anyhow::Result<Vec<(String, crate::driver::AgentInvocation)>> {
        let RunStatus::ParallelAwait { stage, pending_runs } = &self.state.status else {
            anyhow::bail!("not in ParallelAwait");
        };
        let stage_name = stage.clone();
        let stages = self.stages();
        let stage_decl = stages
            .get(stage_name.as_str())
            .ok_or_else(|| anyhow::anyhow!("stage '{}' not found", stage_name))?;
        let tl_path = self.state.line_file.clone();
        let input_artifacts = self.stage_input_artifacts(stage_decl, &self.state.artifacts);
        let stage_runner = stage_decl.runner.clone();

        let runners = self.runners();
        let mut result = Vec::new();
        for run_name in pending_runs {
            let run_decl = stage_decl.runs.iter()
                .find(|r| &r.name == run_name)
                .ok_or_else(|| anyhow::anyhow!("run '{}' not found in stage '{}'", run_name, stage_name))?;

            let effective_runner = run_decl.runner.as_ref().or(stage_runner.as_ref());
            let runner_spec = if let Some(rn) = effective_runner {
                let rd = runners.get(rn.as_str())
                    .ok_or_else(|| anyhow::anyhow!("runner '{}' not found", rn))?;
                self.resolve_runner(rd, &tl_path)?
            } else {
                crate::events::RunnerSpec {
                    name: "default".to_string(), model: None, system: None,
                    tools: vec![], temperature: None, max_tokens: None,
                }
            };

            let prompt = match &run_decl.prompt {
                Some(crate::ast::PromptSource::Inline(s)) => Some(s.clone()),
                Some(crate::ast::PromptSource::File(p)) => {
                    let abs = tl_path.parent().unwrap_or(std::path::Path::new(".")).join(p);
                    Some(std::fs::read_to_string(&abs)
                        .map_err(|_| anyhow::anyhow!("prompt file not found: {}", abs.display()))?)
                }
                None => None,
            };

            result.push((run_name.clone(), crate::driver::AgentInvocation {
                run_id: self.state.run_id.clone(),
                stage: stage_name.clone(),
                runner: runner_spec,
                artifacts: input_artifacts.clone(),
                prompt,
            }));
        }
        Ok(result)
    }

    /// Advance the pipeline one step: invoke the current stage via the driver.
    /// For the stdio driver this emits stage_invoke, saves state, and returns.
    pub async fn advance(&mut self, driver: &dyn Driver) -> anyhow::Result<AdvanceOutcome> {
        let pipeline_name = self.state.pipeline.clone();
        let pipeline = self
            .pipeline(&pipeline_name)
            .ok_or_else(|| anyhow::anyhow!("thruline '{}' not found", pipeline_name))?;

        let (current_stage_name, parallel_hint) = match &self.state.status {
            RunStatus::Running => (pipeline.start.clone(), None),
            RunStatus::AwaitingResume { stage, parallel } => (stage.clone(), *parallel),
            RunStatus::Done | RunStatus::Failed(_) | RunStatus::ParallelAwait { .. } => {
                return Ok(AdvanceOutcome::Idle)
            }
        };

        *self.state.visit_counts.entry(current_stage_name.clone()).or_insert(0) += 1;

        let stages = self.stages();
        let stage = stages
            .get(&current_stage_name)
            .ok_or_else(|| anyhow::anyhow!("stage '{}' not found", current_stage_name))?;

        let tl_path = self.state.line_file.clone();
        let input_artifacts = self.stage_input_artifacts(stage, &self.state.artifacts);

        if !stage.runs.is_empty() {
            // Stage has named run blocks — emit one StageInvoke per run in parallel.
            let runs = stage.runs.clone();
            let stage_runner_name = stage.runner.clone();
            let run_names: Vec<String> = runs.iter().map(|r| r.name.clone()).collect();

            ThrulineEvent::ParallelStart {
                run_id: self.state.run_id.clone(),
                ts: chrono::Utc::now(),
                stage: current_stage_name.clone(),
                count: runs.len(),
                concurrency_limit: None,
            }.emit();

            for run_decl in &runs {
                let effective_runner_name = run_decl.runner.as_ref().or(stage_runner_name.as_ref());
                let runner_spec = if let Some(rn) = effective_runner_name {
                    let runners = self.runners();
                    let rd = runners.get(rn).ok_or_else(|| anyhow::anyhow!("runner '{}' not found", rn))?;
                    self.resolve_runner(rd, &tl_path)?
                } else {
                    RunnerSpec { name: "default".to_string(), model: None, system: None, tools: vec![], temperature: None, max_tokens: None }
                };

                let run_outputs: Vec<OutputDecl> = run_decl.outputs.iter()
                    .map(|a| OutputDecl {
                        name: a.name.clone(),
                        kind: match a.kind { ArtifactKind::Path => "path".to_string(), ArtifactKind::Value => "value".to_string() },
                    })
                    .collect();

                let prompt = match &run_decl.prompt {
                    Some(PromptSource::Inline(s)) => Some(s.clone()),
                    Some(PromptSource::File(p)) => {
                        let abs = tl_path.parent().unwrap_or(Path::new(".")).join(p);
                        Some(std::fs::read_to_string(&abs).map_err(|_| anyhow::anyhow!("prompt file not found: {}", abs.display()))?)
                    }
                    None => None,
                };

                ThrulineEvent::StageInvoke {
                    run_id: self.state.run_id.clone(),
                    ts: chrono::Utc::now(),
                    stage: current_stage_name.clone(),
                    run: Some(run_decl.name.clone()),
                    runner: runner_spec,
                    artifacts: input_artifacts.clone(),
                    prompt,
                    outputs: run_outputs,
                    parallel: None,
                }.emit();
            }

            self.state.status = RunStatus::ParallelAwait {
                stage: current_stage_name.clone(),
                pending_runs: run_names,
            };
            self.state.save()?;
            return Ok(AdvanceOutcome::RunsDispatched { stage: current_stage_name });
        }

        let runner_spec = if let Some(runner_name) = &stage.runner {
            let runners = self.runners();
            let runner_decl = runners
                .get(runner_name)
                .ok_or_else(|| anyhow::anyhow!("runner '{}' not found", runner_name))?;
            self.resolve_runner(runner_decl, &tl_path)?
        } else {
            RunnerSpec { name: "default".to_string(), model: None, system: None, tools: vec![], temperature: None, max_tokens: None }
        };
        let declared_outputs: Vec<OutputDecl> = stage.outputs.iter()
            .map(|a| OutputDecl {
                name: a.name.clone(),
                kind: match a.kind {
                    ArtifactKind::Path  => "path".to_string(),
                    ArtifactKind::Value => "value".to_string(),
                },
            })
            .collect();

        let prompt = match &stage.prompt {
            Some(PromptSource::Inline(s)) => Some(s.clone()),
            Some(PromptSource::File(p)) => {
                let abs = tl_path.parent().unwrap_or(Path::new(".")).join(p);
                Some(std::fs::read_to_string(&abs).map_err(|_| {
                    anyhow::anyhow!("prompt file not found: {}", abs.display())
                })?)
            }
            None => None,
        };

        ThrulineEvent::StageInvoke {
            run_id: self.state.run_id.clone(),
            ts: chrono::Utc::now(),
            stage: current_stage_name.clone(),
            run: None,
            runner: runner_spec.clone(),
            artifacts: input_artifacts.clone(),
            prompt: prompt.clone(),
            outputs: declared_outputs,
            parallel: parallel_hint,
        }
        .emit();

        self.state.status = RunStatus::AwaitingResume {
            stage: current_stage_name.clone(),
            parallel: None,
        };
        self.state.save()?;

        let invocation = AgentInvocation {
            run_id: self.state.run_id.clone(),
            stage: current_stage_name.clone(),
            runner: runner_spec,
            artifacts: input_artifacts,
            prompt,
        };
        let result = driver.invoke_agent(invocation).await?;

        Ok(AdvanceOutcome::Invoked { stage: current_stage_name, result })
    }

    /// Feed completed stage output back into the state machine.
    /// `run_name` is required when the stage is in `ParallelAwait` (run blocks).
    /// Updates artifact store, evaluates routes, saves state.
    pub fn resume_stage(
        &mut self,
        stage_name: &str,
        run_name: Option<&str>,
        outputs: Vec<(String, String, bool)>,
    ) -> anyhow::Result<()> {
        // Handle run-block parallel resumption.
        if let RunStatus::ParallelAwait { stage, pending_runs } = &self.state.status.clone() {
            if stage != stage_name {
                anyhow::bail!(
                    "run is awaiting stage '{}', cannot resume stage '{}'",
                    stage, stage_name
                );
            }
            let run = run_name.ok_or_else(|| anyhow::anyhow!(
                "stage '{}' has parallel runs in progress — use --run <name>", stage_name
            ))?;
            if !pending_runs.contains(&run.to_string()) {
                anyhow::bail!(
                    "run '{}' is not pending for stage '{}' (pending: {:?})",
                    run, stage_name, pending_runs
                );
            }

            // Store artifacts under the stage namespace.
            for (name, value, is_file) in &outputs {
                let key = format!("{}.{}", stage_name, name);
                if *is_file { self.state.artifacts.set_file(&key, value); }
                else { self.state.artifacts.set_ref(&key, value); }
            }

            let remaining: Vec<String> = pending_runs.iter()
                .filter(|r| r.as_str() != run)
                .cloned()
                .collect();

            if remaining.is_empty() {
                // All runs done — emit parallel_done, add to history, route normally.
                ThrulineEvent::ParallelDone {
                    run_id: self.state.run_id.clone(),
                    ts: chrono::Utc::now(),
                    stage: stage_name.to_string(),
                    results: vec![],
                }.emit();
                self.state.history.push(stage_name.to_string());
                self.state.status = RunStatus::Running;  // temporary; overwritten below by routing
                // Fall through to routing logic by re-entering with a synthetic AwaitingResume.
                self.route_after_stage(stage_name)?;
            } else {
                self.state.status = RunStatus::ParallelAwait {
                    stage: stage_name.to_string(),
                    pending_runs: remaining,
                };
                self.state.save()?;
            }
            return Ok(());
        }

        match &self.state.status {
            RunStatus::AwaitingResume { stage, .. } if stage == stage_name => {}
            RunStatus::AwaitingResume { stage, .. } => anyhow::bail!(
                "run is awaiting stage '{}', cannot resume stage '{}'",
                stage, stage_name
            ),
            RunStatus::Done => anyhow::bail!(
                "run '{}' is already done", self.state.run_id
            ),
            RunStatus::Failed(e) => anyhow::bail!(
                "run '{}' has failed: {}", self.state.run_id, e
            ),
            RunStatus::Running => anyhow::bail!(
                "run '{}' has not reached a resume point yet", self.state.run_id
            ),
            RunStatus::ParallelAwait { .. } => unreachable!("handled above"),
        }

        {
            let outputs_json: serde_json::Map<String, serde_json::Value> = outputs.iter()
                .map(|(k, v, _)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            ThrulineEvent::StageComplete {
                run_id: self.state.run_id.clone(),
                ts: chrono::Utc::now(),
                stage: stage_name.to_string(),
                outputs: serde_json::Value::Object(outputs_json),
            }
            .emit();
        }

        for (name, value, is_file) in outputs {
            let key = format!("{}.{}", stage_name, name);
            if is_file {
                self.state.artifacts.set_file(&key, &value);
            } else {
                self.state.artifacts.set_ref(&key, &value);
            }
        }

        self.state.history.push(stage_name.to_string());
        self.route_after_stage(stage_name)?;
        Ok(())
    }

    /// Evaluate routes after a stage completes and update status + save state.
    fn route_after_stage(&mut self, stage_name: &str) -> anyhow::Result<()> {
        let pipeline_name = self.state.pipeline.clone();
        let pipeline = self
            .pipeline(&pipeline_name)
            .ok_or_else(|| anyhow::anyhow!("thruline '{}' not found", pipeline_name))?;

        let routes: Vec<Route> = pipeline.routes.clone();
        let artifacts_snapshot = self.state.artifacts.clone();

        if let Some((route, next_stage)) =
            self.evaluate_routes(stage_name, &routes, &artifacts_snapshot)
        {
            const GLOBAL_MAX_VISITS: u32 = 100;
            let limit = route.max_visits.unwrap_or(GLOBAL_MAX_VISITS);
            let visits = *self.state.visit_counts.get(&next_stage).unwrap_or(&0);
            if visits >= limit {
                anyhow::bail!(
                    "stage '{}' has been visited {} times (limit: {}) — possible infinite loop. \
                     Check your route predicates.",
                    next_stage, visits, limit
                );
            }

            let parallel = route.target.parallel_spec.as_ref().map(|spec| spec.limit);
            let predicate_desc = format_route_source(&route.source);
            ThrulineEvent::RouteTaken {
                run_id: self.state.run_id.clone(),
                ts: chrono::Utc::now(),
                from: stage_name.to_string(),
                to: next_stage.clone(),
                predicate: predicate_desc,
            }.emit();
            self.state.status = RunStatus::AwaitingResume { stage: next_stage, parallel };
        } else {
            ThrulineEvent::PipelineDone {
                run_id: self.state.run_id.clone(),
                ts: chrono::Utc::now(),
                outputs: self.state.artifacts.to_json(),
            }.emit();
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
            system: Some(PromptSource::Inline("system prompt".to_string())),
            tools: vec![],
            temperature: None,
            max_tokens: None,
        })
    }

    fn mk_stage(name: &str, runner_name: &str, outputs: &[(&str, ArtifactKind)]) -> TlItem {
        TlItem::Stage(StageDecl {
            name: name.to_string(),
            inputs: vec![],
            outputs: outputs
                .iter()
                .map(|(n, k)| ArtifactDecl {
                    name: n.to_string(),
                    source: None,
                    optional: false,
                    kind: k.clone(),
                    seed_path: None,
                })
                .collect(),
            runner: Some(runner_name.to_string()),
            prompt: None,
            runs: vec![],
        })
    }

    fn mk_runtime() -> Runtime {
        let state = RunState::new("r1".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            mk_stage("a", "runner", &[("verdict", ArtifactKind::Value)]),
            mk_stage("b", "runner", &[]),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(),
                inputs: vec![],
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
                        max_visits: None,
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
                        max_visits: None,
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
                inputs: vec![],
                start: "x".into(),
                routes: vec![Route {
                    source: RouteSource::Stage("x".into()),
                    target: RouteTarget { stage: "y".into(), parallel_spec: None },
                    max_visits: None,
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
        assert_eq!(spec.system, Some("system prompt".to_string()));
    }

    #[test]
    fn test_resume_stage_updates_artifacts_and_advances() {
        let mut rt = mk_runtime();
        rt.state.status = crate::runtime::state::RunStatus::AwaitingResume { stage: "a".into(), parallel: None };
        rt.resume_stage("a", None, vec![("verdict".to_string(), "ok".to_string(), false)]).unwrap();
        assert!(matches!(&rt.state.status, RunStatus::AwaitingResume { stage, .. } if stage == "b"));
        assert_eq!(rt.state.artifacts.get_ref("a.verdict"), Some("ok"));
    }

    #[test]
    fn test_stage_input_artifacts_history_fallback() {
        // Stage b declares in: verdict — should find a.verdict via history lookup
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            TlItem::Stage(StageDecl {
                name: "a".into(),
                inputs: vec![],
                outputs: vec![ArtifactDecl { name: "verdict".into(), source: None, optional: false, kind: ArtifactKind::Value, seed_path: None }],
                runner: Some("runner".into()),
                prompt: None,
                runs: vec![],
            }),
            TlItem::Stage(StageDecl {
                name: "b".into(),
                inputs: vec![ArtifactDecl { name: "verdict".into(), source: None, optional: false, kind: ArtifactKind::Value, seed_path: None }],
                outputs: vec![],
                runner: Some("runner".into()),
                prompt: None,
                runs: vec![],
            }),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(),
                inputs: vec![],
                start: "a".into(),
                routes: vec![Route {
                    source: RouteSource::Stage("a".into()),
                    target: RouteTarget { stage: "b".into(), parallel_spec: None },
                    max_visits: None,
                }],
            }),
        ];
        let mut rt = Runtime::new(state, items);

        // Simulate a completing — stores a.verdict, adds to history
        rt.state.artifacts.set_ref("a.verdict", "approved");
        rt.state.history.push("a".to_string());

        let stages = rt.stages();
        let stage_b = stages.get("b").unwrap();
        let result = rt.stage_input_artifacts(stage_b, &rt.state.artifacts.clone());

        // b.verdict and input.verdict are both absent — should find a.verdict via history
        assert_eq!(result["verdict"], "approved");
    }

    #[test]
    fn test_stage_input_artifacts_history_prefers_most_recent() {
        // If two prior stages both produced the same artifact name, newest wins
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            mk_stage("a", "runner", &[("score", ArtifactKind::Value)]),
            mk_stage("b", "runner", &[("score", ArtifactKind::Value)]),
            TlItem::Stage(StageDecl {
                name: "c".into(),
                inputs: vec![ArtifactDecl { name: "score".into(), source: None, optional: false, kind: ArtifactKind::Value, seed_path: None }],
                outputs: vec![],
                runner: Some("runner".into()),
                prompt: None,
                runs: vec![],
            }),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(),
                inputs: vec![],
                start: "a".into(),
                routes: vec![],
            }),
        ];
        let mut rt = Runtime::new(state, items);

        rt.state.artifacts.set_ref("a.score", "first");
        rt.state.artifacts.set_ref("b.score", "second");
        rt.state.history = vec!["a".to_string(), "b".to_string()];

        let stages = rt.stages();
        let stage_c = stages.get("c").unwrap();
        let result = rt.stage_input_artifacts(stage_c, &rt.state.artifacts.clone());

        // b ran after a, so b.score wins
        assert_eq!(result["score"], "second");
    }

    #[test]
    fn test_stage_input_artifacts_own_key_beats_history() {
        // stage.artifact takes precedence over history lookup
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            mk_stage("a", "runner", &[("note", ArtifactKind::Value)]),
            TlItem::Stage(StageDecl {
                name: "b".into(),
                inputs: vec![ArtifactDecl { name: "note".into(), source: None, optional: false, kind: ArtifactKind::Value, seed_path: None }],
                outputs: vec![],
                runner: Some("runner".into()),
                prompt: None,
                runs: vec![],
            }),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(),
                inputs: vec![],
                start: "a".into(),
                routes: vec![],
            }),
        ];
        let mut rt = Runtime::new(state, items);

        rt.state.artifacts.set_ref("a.note", "from-history");
        rt.state.artifacts.set_ref("b.note", "own-output");
        rt.state.history = vec!["a".to_string()];

        let stages = rt.stages();
        let stage_b = stages.get("b").unwrap();
        let result = rt.stage_input_artifacts(stage_b, &rt.state.artifacts.clone());

        // b.note exists directly — should not fall through to history
        assert_eq!(result["note"], "own-output");
    }

    #[test]
    fn test_stage_input_qualified_source_direct_lookup() {
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            mk_stage("classify", "runner", &[("lang", ArtifactKind::Value)]),
            mk_stage("revise",   "runner", &[("lang", ArtifactKind::Value)]),
            TlItem::Stage(StageDecl {
                name: "analyze".into(),
                inputs: vec![ArtifactDecl {
                    name: "lang".into(),
                    source: Some("classify".into()),  // explicit: always use classify.lang
                    optional: false,
                    kind: ArtifactKind::Value,
                    seed_path: None,
                }],
                outputs: vec![],
                runner: Some("runner".into()),
                prompt: None,
                runs: vec![],
            }),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(),
                inputs: vec![],
                start: "classify".into(),
                routes: vec![],
            }),
        ];
        let mut rt = Runtime::new(state, items);

        rt.state.artifacts.set_ref("classify.lang", "python");
        rt.state.artifacts.set_ref("revise.lang", "rust");  // newer in history
        rt.state.history = vec!["classify".to_string(), "revise".to_string()];

        let stages = rt.stages();
        let stage_analyze = stages.get("analyze").unwrap();
        let result = rt.stage_input_artifacts(stage_analyze, &rt.state.artifacts.clone());

        // Explicit source: should use classify.lang, not revise.lang (even though revise is newer)
        assert_eq!(result["lang"], "python");
    }

    #[test]
    fn test_resume_wrong_stage_errors() {
        let mut rt = mk_runtime();
        rt.state.status = RunStatus::AwaitingResume { stage: "a".into(), parallel: None };
        let err = rt.resume_stage("b", None, vec![]).unwrap_err();
        assert!(err.to_string().contains("awaiting stage 'a'"), "got: {}", err);
    }

    #[test]
    fn test_resume_done_run_errors() {
        let mut rt = mk_runtime();
        rt.state.status = RunStatus::Done;
        let err = rt.resume_stage("a", None, vec![]).unwrap_err();
        assert!(err.to_string().contains("already done"), "got: {}", err);
    }

    #[test]
    fn test_seed_path_used_when_no_artifact_present() {
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            TlItem::Stage(StageDecl {
                name: "analyze".into(),
                inputs: vec![ArtifactDecl {
                    name: "brief".into(), source: None, optional: true,
                    kind: ArtifactKind::Path,
                    seed_path: Some("specs/brief.md".into()),
                }],
                outputs: vec![], runner: Some("runner".into()),
                prompt: None, runs: vec![],
            }),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(), inputs: vec![], start: "analyze".into(), routes: vec![],
            }),
        ];
        let rt = Runtime::new(state, items);
        let stages = rt.stages();
        let stage = stages.get("analyze").unwrap();
        let result = rt.stage_input_artifacts(stage, &rt.state.artifacts.clone());
        assert!(
            result["brief"].as_str().unwrap_or("").ends_with("specs/brief.md"),
            "seed path not applied: {}", result
        );
    }

    #[test]
    fn test_format_route_source_predicate() {
        assert_eq!(
            format_route_source(&RouteSource::Predicate {
                stage: "review".into(),
                artifact: "verdict".into(),
                op: CompareOp::Eq,
                value: "approved".into(),
            }),
            r#"review.verdict == "approved""#
        );
        assert_eq!(format_route_source(&RouteSource::Stage("a".into())), "Stage(a)");
        assert_eq!(format_route_source(&RouteSource::FanIn("workers".into())), "FanIn(workers)");
        assert_eq!(
            format_route_source(&RouteSource::Predicate {
                stage: "a".into(), artifact: "v".into(),
                op: CompareOp::Ne, value: "rejected".into(),
            }),
            r#"a.v != "rejected""#
        );
    }

    #[test]
    fn test_fan_out_route_sets_parallel_hint_on_next_status() {
        // Fan-out [*2] route should propagate parallel: Some(Some(2)) onto AwaitingResume
        let state = RunState::new("r".into(), "p3".into(), "/tmp/t.line".into());
        let items = vec![
            mk_runner("r"),
            mk_stage("a", "r", &[]),
            mk_stage("b", "r", &[]),
            mk_stage("c", "r", &[]),
            TlItem::Pipeline(PipelineDecl {
                name: "p3".into(),
                inputs: vec![],
                start: "a".into(),
                routes: vec![
                    Route {
                        source: RouteSource::Stage("a".into()),
                        target: RouteTarget {
                            stage: "b".into(),
                            parallel_spec: Some(ParallelSpec { limit: Some(2) }),
                        },
                        max_visits: None,
                    },
                    Route {
                        source: RouteSource::FanIn("b".into()),
                        target: RouteTarget { stage: "c".into(), parallel_spec: None },
                        max_visits: None,
                    },
                ],
            }),
        ];
        let mut rt = Runtime::new(state, items);
        rt.state.status = RunStatus::AwaitingResume { stage: "a".into(), parallel: None };
        rt.resume_stage("a", None, vec![]).unwrap();
        // b should be next, with parallel hint Some(Some(2)) = [*2]
        assert!(matches!(
            &rt.state.status,
            RunStatus::AwaitingResume { stage, parallel } if stage == "b" && *parallel == Some(Some(2))
        ), "expected AwaitingResume{{b, parallel=Some(Some(2))}}, got: {:?}", rt.state.status);
    }

    #[test]
    fn test_unlimited_fan_out_sets_none_limit_hint() {
        // Fan-out [*] (no limit) should set parallel: Some(None)
        let state = RunState::new("r".into(), "p4".into(), "/tmp/t.line".into());
        let items = vec![
            mk_runner("r"),
            mk_stage("a", "r", &[]),
            mk_stage("b", "r", &[]),
            TlItem::Pipeline(PipelineDecl {
                name: "p4".into(),
                inputs: vec![],
                start: "a".into(),
                routes: vec![Route {
                    source: RouteSource::Stage("a".into()),
                    target: RouteTarget {
                        stage: "b".into(),
                        parallel_spec: Some(ParallelSpec { limit: None }),
                    },
                    max_visits: None,
                }],
            }),
        ];
        let mut rt = Runtime::new(state, items);
        rt.state.status = RunStatus::AwaitingResume { stage: "a".into(), parallel: None };
        rt.resume_stage("a", None, vec![]).unwrap();
        assert!(matches!(
            &rt.state.status,
            RunStatus::AwaitingResume { stage, parallel } if stage == "b" && *parallel == Some(None)
        ), "expected AwaitingResume{{b, parallel=Some(None)}}, got: {:?}", rt.state.status);
    }

    #[test]
    fn test_history_beats_pipeline_input_for_same_artifact() {
        // Revision loop: input.code is the original; revise.code is newer.
        // Unqualified `in: code` should return revise.code (history), not input.code.
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            mk_stage("revise", "runner", &[("code", ArtifactKind::Value)]),
            TlItem::Stage(StageDecl {
                name: "analyze".into(),
                inputs: vec![ArtifactDecl {
                    name: "code".into(), source: None, optional: false,
                    kind: ArtifactKind::Value, seed_path: None,
                }],
                outputs: vec![],
                runner: Some("runner".into()),
                prompt: None, runs: vec![],
            }),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(), inputs: vec![], start: "revise".into(), routes: vec![],
            }),
        ];
        let mut rt = Runtime::new(state, items);
        rt.state.artifacts.set_ref("input.code", "original");
        rt.state.artifacts.set_ref("revise.code", "revised");
        rt.state.history = vec!["revise".to_string()];

        let stages = rt.stages();
        let stage = stages.get("analyze").unwrap();
        let result = rt.stage_input_artifacts(stage, &rt.state.artifacts.clone());
        assert_eq!(result["code"], "revised", "history should beat pipeline input: {}", result);
    }

    #[test]
    fn test_input_source_pins_to_pipeline_input_despite_history() {
        // Explicit `in: input.code as value` always returns pipeline input, ignoring history.
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            mk_stage("revise", "runner", &[("code", ArtifactKind::Value)]),
            TlItem::Stage(StageDecl {
                name: "analyze".into(),
                inputs: vec![ArtifactDecl {
                    name: "code".into(), source: Some("input".into()), optional: false,
                    kind: ArtifactKind::Value, seed_path: None,
                }],
                outputs: vec![],
                runner: Some("runner".into()),
                prompt: None, runs: vec![],
            }),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(), inputs: vec![], start: "revise".into(), routes: vec![],
            }),
        ];
        let mut rt = Runtime::new(state, items);
        rt.state.artifacts.set_ref("input.code", "original");
        rt.state.artifacts.set_ref("revise.code", "revised");
        rt.state.history = vec!["revise".to_string()];

        let stages = rt.stages();
        let stage = stages.get("analyze").unwrap();
        let result = rt.stage_input_artifacts(stage, &rt.state.artifacts.clone());
        assert_eq!(result["code"], "original", "explicit input.code should pin to pipeline input: {}", result);
    }

    #[test]
    fn test_pipeline_input_used_when_no_history_matches() {
        // Without any history producing `code`, input.code should still be found.
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            TlItem::Stage(StageDecl {
                name: "analyze".into(),
                inputs: vec![ArtifactDecl {
                    name: "code".into(), source: None, optional: false,
                    kind: ArtifactKind::Value, seed_path: None,
                }],
                outputs: vec![],
                runner: Some("runner".into()),
                prompt: None, runs: vec![],
            }),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(), inputs: vec![], start: "analyze".into(), routes: vec![],
            }),
        ];
        let mut rt = Runtime::new(state, items);
        rt.state.artifacts.set_ref("input.code", "seed-value");
        // No history, no stage output

        let stages = rt.stages();
        let stage = stages.get("analyze").unwrap();
        let result = rt.stage_input_artifacts(stage, &rt.state.artifacts.clone());
        assert_eq!(result["code"], "seed-value", "pipeline input should be used as fallback: {}", result);
    }

    #[test]
    fn test_run_blocks_emit_parallel_start_and_multiple_stage_invokes() {
        use crate::runtime::state::RunStatus;
        use tokio::runtime::Runtime as TokioRuntime;

        let state = RunState::new("r1".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("analyst"),
            mk_runner("critic"),
            TlItem::Stage(StageDecl {
                name: "review".into(),
                inputs: vec![],
                outputs: vec![],
                runner: Some("analyst".into()),
                prompt: None,
                runs: vec![
                    RunDecl {
                        name: "quality".into(),
                        runner: None,  // inherits stage runner
                        prompt: Some(PromptSource::Inline("Check quality.".into())),
                        outputs: vec![ArtifactDecl {
                            name: "verdict".into(), source: None, optional: false,
                            kind: ArtifactKind::Value, seed_path: None,
                        }],
                    },
                    RunDecl {
                        name: "risk".into(),
                        runner: Some("critic".into()),
                        prompt: Some(PromptSource::Inline("Check risks.".into())),
                        outputs: vec![ArtifactDecl {
                            name: "risks".into(), source: None, optional: false,
                            kind: ArtifactKind::Value, seed_path: None,
                        }],
                    },
                ],
            }),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(), inputs: vec![], start: "review".into(), routes: vec![],
            }),
        ];
        let mut rt = Runtime::new(state, items);
        let driver = crate::driver::stdio::StdioDriver;
        TokioRuntime::new().unwrap().block_on(rt.advance(&driver)).unwrap();

        assert!(matches!(
            &rt.state.status,
            RunStatus::ParallelAwait { stage, pending_runs }
            if stage == "review" && pending_runs.len() == 2
                && pending_runs.contains(&"quality".to_string())
                && pending_runs.contains(&"risk".to_string())
        ), "expected ParallelAwait with 2 pending runs, got {:?}", rt.state.status);
    }

    #[test]
    fn test_run_blocks_parallel_resume_completes_stage_on_last_run() {
        use crate::runtime::state::RunStatus;

        let state = RunState::new("r1".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("analyst"),
            TlItem::Stage(StageDecl {
                name: "review".into(),
                inputs: vec![],
                outputs: vec![],
                runner: Some("analyst".into()),
                prompt: None,
                runs: vec![
                    RunDecl {
                        name: "quality".into(),
                        runner: None,
                        prompt: Some(PromptSource::Inline("Check quality.".into())),
                        outputs: vec![ArtifactDecl {
                            name: "verdict".into(), source: None, optional: false,
                            kind: ArtifactKind::Value, seed_path: None,
                        }],
                    },
                    RunDecl {
                        name: "risk".into(),
                        runner: None,
                        prompt: Some(PromptSource::Inline("Check risks.".into())),
                        outputs: vec![ArtifactDecl {
                            name: "risks".into(), source: None, optional: false,
                            kind: ArtifactKind::Value, seed_path: None,
                        }],
                    },
                ],
            }),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(), inputs: vec![], start: "review".into(), routes: vec![],
            }),
        ];
        let mut rt = Runtime::new(state, items);
        rt.state.status = RunStatus::ParallelAwait {
            stage: "review".into(),
            pending_runs: vec!["quality".to_string(), "risk".to_string()],
        };

        // First run completes — still waiting.
        rt.resume_stage("review", Some("quality"), vec![("verdict".into(), "approved".into(), false)]).unwrap();
        assert!(matches!(&rt.state.status, RunStatus::ParallelAwait { pending_runs, .. } if pending_runs.len() == 1));
        assert_eq!(rt.state.artifacts.get_ref("review.verdict"), Some("approved"));

        // Second run completes — stage done, pipeline done.
        rt.resume_stage("review", Some("risk"), vec![("risks".into(), "none".into(), false)]).unwrap();
        assert!(matches!(&rt.state.status, RunStatus::Done));
        assert_eq!(rt.state.artifacts.get_ref("review.risks"), Some("none"));
        assert!(rt.state.history.contains(&"review".to_string()));
    }

    #[test]
    fn test_visit_count_increments_on_advance() {
        use tokio::runtime::Runtime as TokioRuntime;
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            mk_stage("a", "runner", &[("verdict", ArtifactKind::Value)]),
            mk_stage("b", "runner", &[]),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(), inputs: vec![], start: "a".into(), routes: vec![],
            }),
        ];
        let mut rt = Runtime::new(state, items);
        let driver = crate::driver::stdio::StdioDriver;
        TokioRuntime::new().unwrap().block_on(rt.advance(&driver)).unwrap();
        assert_eq!(rt.state.visit_counts.get("a").copied().unwrap_or(0), 1,
            "visit count should be 1 after one advance");
    }

    #[test]
    fn test_loop_limit_error_at_100_visits() {
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            mk_stage("a", "runner", &[("verdict", ArtifactKind::Value)]),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(), inputs: vec![], start: "a".into(),
                routes: vec![Route {
                    source: RouteSource::Stage("a".into()),
                    target: RouteTarget { stage: "a".into(), parallel_spec: None },
                    max_visits: None,
                }],
            }),
        ];
        let mut rt = Runtime::new(state, items);
        rt.state.visit_counts.insert("a".to_string(), 100);
        rt.state.status = RunStatus::AwaitingResume { stage: "a".into(), parallel: None };
        let err = rt.resume_stage("a", None, vec![]).unwrap_err();
        assert!(err.to_string().contains("visited"),
            "expected loop limit error, got: {}", err);
    }

    #[test]
    fn test_per_route_max_visits_overrides_global() {
        let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
        let items = vec![
            mk_runner("runner"),
            mk_stage("a", "runner", &[]),
            TlItem::Pipeline(PipelineDecl {
                name: "p".into(), inputs: vec![], start: "a".into(),
                routes: vec![Route {
                    source: RouteSource::Stage("a".into()),
                    target: RouteTarget { stage: "a".into(), parallel_spec: None },
                    max_visits: Some(3),
                }],
            }),
        ];
        let mut rt = Runtime::new(state, items);
        rt.state.visit_counts.insert("a".to_string(), 3);
        rt.state.status = RunStatus::AwaitingResume { stage: "a".into(), parallel: None };
        let err = rt.resume_stage("a", None, vec![]).unwrap_err();
        assert!(err.to_string().contains("3"), "expected limit 3 in error: {}", err);
        assert!(err.to_string().contains("visited"), "expected 'visited' in error: {}", err);
    }
}
