# Runtime Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 6 runtime correctness bugs: API driver multi-stage loop (B1), run blocks in API mode (B2), silent empty output as stage_error (B3), infinite loop safety via visit counts (B4), seed paths applied at runtime (B5), protocol version field on pipeline_start (B6).

**Architecture:** `advance()` return type changes from `bool` to `AdvanceOutcome` enum carrying `AgentResult`; `cmd_run` API branch becomes a proper loop using that result and handling run blocks sequentially; `RunState` gains `visit_counts`; `PipelineStart` event gains a `protocol` field.

**Tech Stack:** Rust, existing `anyhow`/`serde`/`chrono`; no new dependencies.

---

## Files

- Modify: `src/runtime/mod.rs` — `AdvanceOutcome` enum, `advance()` return type, `pending_run_invocations()`, visit tracking, seed path fallback
- Modify: `src/runtime/state.rs` — add `visit_counts: HashMap<String, u32>` to `RunState`
- Modify: `src/events.rs` — add `protocol: String` to `PipelineStart`
- Modify: `src/cli.rs` — API driver loop, all `advance()` call sites, `PipelineStart` protocol field
- Modify: `tests/integration.rs` — protocol assertion
- Modify: `docs/LANGUAGE.md` — remove seed paths from Feature Gaps, update event table

---

### Task 1: Add `AdvanceOutcome` and change `advance()` return type

**Files:**
- Modify: `src/runtime/mod.rs`
- Modify: `src/cli.rs` (fix call sites)

- [ ] **Step 1: Add `AdvanceOutcome` and `AgentResult` import to `src/runtime/mod.rs`**

Add after the existing `use` statements at the top of `src/runtime/mod.rs`:

```rust
use crate::driver::AgentResult;

/// Outcome of one `advance()` call.
pub enum AdvanceOutcome {
    /// A regular stage was invoked. `result` is the driver output (empty for stdio).
    Invoked { stage: String, result: AgentResult },
    /// The stage had `run` blocks. `ParallelAwait` is now set.
    /// API driver must call `pending_run_invocations()` to drive each run.
    RunsDispatched { stage: String },
    /// Pipeline is Done/Failed/already ParallelAwait — nothing to advance.
    Idle,
}
```

- [ ] **Step 2: Change `advance()` signature**

In `src/runtime/mod.rs`, change:
```rust
pub async fn advance(&mut self, driver: &dyn Driver) -> anyhow::Result<bool> {
```
to:
```rust
pub async fn advance(&mut self, driver: &dyn Driver) -> anyhow::Result<AdvanceOutcome> {
```

- [ ] **Step 3: Update the three return sites inside `advance()`**

*Return 1* — idle guard at the top of the match:
```rust
RunStatus::Done | RunStatus::Failed(_) | RunStatus::ParallelAwait { .. } => {
    return Ok(AdvanceOutcome::Idle)
}
```

*Return 2* — run-block early return (end of run-block branch, after `self.state.save()?`):
```rust
return Ok(AdvanceOutcome::RunsDispatched { stage: current_stage_name });
```

*Return 3* — normal stage invocation, replace the final two lines:
```rust
// was:
driver.invoke_agent(invocation).await?;
Ok(true)

// now:
let result = driver.invoke_agent(invocation).await?;
Ok(AdvanceOutcome::Invoked { stage: current_stage_name, result })
```

- [ ] **Step 4: Fix `cmd_run` stdio call site in `src/cli.rs`**

Replace the `if let Err(e) = runtime.advance(&driver).await` block in the `"stdio"` branch with:

```rust
match runtime.advance(&driver).await {
    Err(e) => {
        crate::events::ThrulineEvent::PipelineError {
            run_id: run_id.clone(), ts: chrono::Utc::now(),
            stage: pending_stage, error: e.to_string(),
        }.emit();
        return Err(e);
    }
    Ok(_) => {} // stdio: harness handles invocation externally
}
```

- [ ] **Step 5: Fix `cmd_resume` advance call site in `src/cli.rs`**

Replace `if let Err(e) = runtime.advance(&driver).await` in `cmd_resume` with:

```rust
match runtime.advance(&driver).await {
    Err(e) => {
        crate::events::ThrulineEvent::PipelineError {
            run_id: run_id.to_string(), ts: chrono::Utc::now(),
            stage: pending_stage, error: e.to_string(),
        }.emit();
        return Err(e);
    }
    Ok(_) => {}
}
```

- [ ] **Step 6: Compile check**

```bash
cargo build 2>&1 | grep '^error'
```
Expected: no errors.

- [ ] **Step 7: Run tests**

```bash
cargo test 2>&1 | tail -3
```
Expected: 94 passed.

- [ ] **Step 8: Commit**

```bash
git add src/runtime/mod.rs src/cli.rs
git commit -m "refactor: advance() returns AdvanceOutcome carrying AgentResult"
```

---

### Task 2: API driver full-pipeline loop (B1 + B2 + B3)

**Files:**
- Modify: `src/runtime/mod.rs` — add `pending_run_invocations()`
- Modify: `src/cli.rs` — replace `"api"` branch with a loop

- [ ] **Step 1: Add `pending_run_invocations()` to `Runtime` in `src/runtime/mod.rs`**

Add this method inside `impl Runtime`, after `check_pipeline_inputs`:

```rust
/// Returns (run_name, AgentInvocation) for each run still pending in ParallelAwait.
/// Used by the API driver to sequentially drive run blocks.
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

    let mut result = Vec::new();
    for run_name in pending_runs {
        let run_decl = stage_decl.runs.iter()
            .find(|r| &r.name == run_name)
            .ok_or_else(|| anyhow::anyhow!("run '{}' not found in stage '{}'", run_name, stage_name))?;

        let effective_runner = run_decl.runner.as_ref().or(stage_runner.as_ref());
        let runner_spec = if let Some(rn) = effective_runner {
            let runners = self.runners();
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
                let abs = tl_path.parent().unwrap_or(Path::new(".")).join(p);
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
```

- [ ] **Step 2: Replace `"api"` branch in `cmd_run` with a full loop**

In `src/cli.rs`, replace the entire `"api" => { ... }` block with:

```rust
"api" => {
    let driver = crate::driver::api::ApiDriver::from_env(config_model)?;
    crate::events::ThrulineEvent::PipelineStart {
        run_id: run_id.clone(),
        ts: chrono::Utc::now(),
        pipeline: pipeline.clone(),
        protocol: "1".to_string(),
        inputs: serde_json::Value::Null,
    }.emit();

    loop {
        // Snapshot pending stage before advancing (needed for error events).
        let pending_stage = match &runtime.state.status {
            RunStatus::Running => runtime.items.iter().find_map(|i| {
                if let crate::ast::TlItem::Pipeline(p) = i {
                    if p.name == runtime.state.pipeline { Some(p.start.clone()) } else { None }
                } else { None }
            }).unwrap_or_else(|| "unknown".to_string()),
            RunStatus::AwaitingResume { stage, .. } => stage.clone(),
            RunStatus::ParallelAwait { stage, .. } => stage.clone(),
            RunStatus::Done | RunStatus::Failed(_) => break,
        };

        match runtime.advance(&driver).await {
            Err(e) => {
                crate::events::ThrulineEvent::PipelineError {
                    run_id: run_id.clone(), ts: chrono::Utc::now(),
                    stage: pending_stage, error: e.to_string(),
                }.emit();
                return Err(e);
            }
            Ok(crate::runtime::AdvanceOutcome::Idle) => break,

            Ok(crate::runtime::AdvanceOutcome::Invoked { stage, result }) => {
                // B3: non-JSON model response produces empty outputs — treat as stage_error.
                if result.outputs.is_empty() {
                    let msg = format!(
                        "stage '{}' returned no parseable outputs — model response was not valid JSON",
                        stage
                    );
                    crate::events::ThrulineEvent::StageError {
                        run_id: run_id.clone(), ts: chrono::Utc::now(),
                        stage: stage.clone(), error: msg.clone(),
                    }.emit();
                    crate::events::ThrulineEvent::PipelineError {
                        run_id: run_id.clone(), ts: chrono::Utc::now(),
                        stage, error: msg.clone(),
                    }.emit();
                    anyhow::bail!("{}", msg);
                }
                if let Err(e) = runtime.resume_stage(&stage, None, result.outputs) {
                    crate::events::ThrulineEvent::StageError {
                        run_id: run_id.clone(), ts: chrono::Utc::now(),
                        stage: stage.clone(), error: e.to_string(),
                    }.emit();
                    crate::events::ThrulineEvent::PipelineError {
                        run_id: run_id.clone(), ts: chrono::Utc::now(),
                        stage, error: e.to_string(),
                    }.emit();
                    return Err(e);
                }
            }

            Ok(crate::runtime::AdvanceOutcome::RunsDispatched { stage }) => {
                // B2: run blocks — call each pending run's API sequentially.
                let run_invocations = match runtime.pending_run_invocations() {
                    Ok(v) => v,
                    Err(e) => {
                        crate::events::ThrulineEvent::PipelineError {
                            run_id: run_id.clone(), ts: chrono::Utc::now(),
                            stage: stage.clone(), error: e.to_string(),
                        }.emit();
                        return Err(e);
                    }
                };
                for (run_name, invocation) in run_invocations {
                    match driver.invoke_agent(invocation).await {
                        Err(e) => {
                            crate::events::ThrulineEvent::StageError {
                                run_id: run_id.clone(), ts: chrono::Utc::now(),
                                stage: stage.clone(), error: e.to_string(),
                            }.emit();
                            crate::events::ThrulineEvent::PipelineError {
                                run_id: run_id.clone(), ts: chrono::Utc::now(),
                                stage: stage.clone(), error: e.to_string(),
                            }.emit();
                            return Err(e);
                        }
                        Ok(result) => {
                            if result.outputs.is_empty() {
                                let msg = format!(
                                    "run '{}.{}' returned no parseable outputs",
                                    stage, run_name
                                );
                                crate::events::ThrulineEvent::StageError {
                                    run_id: run_id.clone(), ts: chrono::Utc::now(),
                                    stage: stage.clone(), error: msg.clone(),
                                }.emit();
                                crate::events::ThrulineEvent::PipelineError {
                                    run_id: run_id.clone(), ts: chrono::Utc::now(),
                                    stage: stage.clone(), error: msg,
                                }.emit();
                                anyhow::bail!("run block returned no parseable outputs");
                            }
                            if let Err(e) = runtime.resume_stage(&stage, Some(&run_name), result.outputs) {
                                crate::events::ThrulineEvent::PipelineError {
                                    run_id: run_id.clone(), ts: chrono::Utc::now(),
                                    stage: stage.clone(), error: e.to_string(),
                                }.emit();
                                return Err(e);
                            }
                        }
                    }
                }
            }
        }

        if matches!(runtime.state.status, RunStatus::Done | RunStatus::Failed(_)) {
            break;
        }
    }
}
```

- [ ] **Step 3: Compile and test**

```bash
cargo test 2>&1 | tail -3
```
Expected: 94 passed.

- [ ] **Step 4: Commit**

```bash
git add src/runtime/mod.rs src/cli.rs
git commit -m "feat: API driver drives full multi-stage pipeline; run blocks sequential in api mode; empty output = stage_error (B1+B2+B3)"
```

---

### Task 3: Loop safety — visit count tracking (B4)

**Files:**
- Modify: `src/runtime/state.rs`
- Modify: `src/runtime/mod.rs`

- [ ] **Step 1: Add `visit_counts` to `RunState` in `src/runtime/state.rs`**

Add `use std::collections::HashMap;` at the top if not present.

In the `RunState` struct, add:
```rust
#[serde(default)]
pub visit_counts: HashMap<String, u32>,
```

In `RunState::new()`, add:
```rust
visit_counts: HashMap::new(),
```

- [ ] **Step 2: Increment on every `advance()` in `src/runtime/mod.rs`**

Inside `advance()`, immediately after the line:
```rust
let (current_stage_name, parallel_hint) = match &self.state.status { ... };
```

Add:
```rust
*self.state.visit_counts.entry(current_stage_name.clone()).or_insert(0) += 1;
```

- [ ] **Step 3: Enforce limit in `route_after_stage()`**

In `route_after_stage()`, inside the `if let Some((route, next_stage)) =` block, add before the `RouteTaken` emit:

```rust
const MAX_STAGE_VISITS: u32 = 100;
let visits = *self.state.visit_counts.get(&next_stage).unwrap_or(&0);
if visits >= MAX_STAGE_VISITS {
    anyhow::bail!(
        "stage '{}' has been visited {} times — possible infinite loop. \
         Check your route predicates.",
        next_stage, visits
    );
}
```

- [ ] **Step 4: Write tests**

In `src/runtime/mod.rs` tests, add:

```rust
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
```

- [ ] **Step 5: Run tests**

```bash
cargo test 2>&1 | tail -3
```
Expected: 96 passed.

- [ ] **Step 6: Commit**

```bash
git add src/runtime/state.rs src/runtime/mod.rs
git commit -m "feat: loop-limit enforcement — bail after 100 visits to same stage (B4)"
```

---

### Task 4: Apply seed paths at runtime (B5)

**Files:**
- Modify: `src/runtime/mod.rs`
- Modify: `docs/LANGUAGE.md`

- [ ] **Step 1: Add seed path fallback in `stage_input_artifacts()`**

In `src/runtime/mod.rs`, in `stage_input_artifacts()`, find the unqualified-resolution block. It ends with:

```rust
found.or_else(|| {
    if let Some(path) = artifacts.get_file(&input_key) {
        Some(serde_json::Value::String(path.to_string_lossy().into_owned()))
    } else {
        artifacts.get_ref(&input_key).map(|v| serde_json::Value::String(v.to_string()))
    }
})
```

Replace with:

```rust
found
    .or_else(|| {
        if let Some(path) = artifacts.get_file(&input_key) {
            Some(serde_json::Value::String(path.to_string_lossy().into_owned()))
        } else {
            artifacts.get_ref(&input_key).map(|v| serde_json::Value::String(v.to_string()))
        }
    })
    .or_else(|| {
        // Seed path: declared default for optional path artifacts when nothing else resolves.
        input.seed_path.as_ref().map(|seed| {
            let abs = self.state.line_file
                .parent()
                .unwrap_or(Path::new("."))
                .join(seed);
            serde_json::Value::String(abs.to_string_lossy().into_owned())
        })
    })
```

- [ ] **Step 2: Write a test**

```rust
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
```

- [ ] **Step 3: Remove seed paths from Feature Gaps in `docs/LANGUAGE.md`**

Delete the `**Seed paths**` entry from the `## Feature Gaps` section.

- [ ] **Step 4: Run tests**

```bash
cargo test 2>&1 | tail -3
```
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/runtime/mod.rs docs/LANGUAGE.md
git commit -m "feat: apply seed paths as fallback for unresolved optional path artifacts (B5)"
```

---

### Task 5: Protocol version on `pipeline_start` (B6)

**Files:**
- Modify: `src/events.rs`
- Modify: `src/cli.rs`
- Modify: `tests/integration.rs`

- [ ] **Step 1: Add `protocol` to `PipelineStart` in `src/events.rs`**

Add a free function above `impl ThrulineEvent`:
```rust
fn default_protocol_version() -> String { "1".to_string() }
```

In the `PipelineStart` variant, add after `pipeline: String,`:
```rust
/// Protocol version — increment when event shapes change incompatibly.
#[serde(default = "default_protocol_version")]
protocol: String,
```

- [ ] **Step 2: Update `test_pipeline_start_serializes` in `src/events.rs`**

```rust
let ev = ThrulineEvent::PipelineStart {
    run_id: "abc".to_string(),
    ts: Utc::now(),
    pipeline: "feature-dev".to_string(),
    protocol: "1".to_string(),
    inputs: serde_json::Value::Null,
};
let s = serde_json::to_string(&ev).unwrap();
assert!(s.contains(r#""event":"pipeline_start""#));
assert!(s.contains(r#""pipeline":"feature-dev""#));
assert!(s.contains(r#""protocol":"1""#));
```

- [ ] **Step 3: Add `protocol` to the stdio `PipelineStart` emit in `src/cli.rs`**

In `cmd_run`'s `"stdio"` branch, update:
```rust
crate::events::ThrulineEvent::PipelineStart {
    run_id: run_id.clone(),
    ts: chrono::Utc::now(),
    pipeline: pipeline.clone(),
    protocol: "1".to_string(),
    inputs: serde_json::Value::Null,
}.emit();
```

(The API branch was updated in Task 2.)

- [ ] **Step 4: Assert protocol in `test_run_stdio_emits_pipeline_start_and_stage_invoke`**

In `tests/integration.rs`, add:
```rust
assert!(stdout.contains(r#""protocol":"1""#), "got: {}", stdout);
```

- [ ] **Step 5: Run all tests**

```bash
cargo test 2>&1 | tail -3
```
Expected: all pass.

- [ ] **Step 6: Push to main**

```bash
git add src/events.rs src/cli.rs tests/integration.rs
git commit -m "feat: protocol version field on pipeline_start event (B6)"
git push origin main
```
