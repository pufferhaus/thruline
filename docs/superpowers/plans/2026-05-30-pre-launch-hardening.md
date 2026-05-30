# Pre-Launch Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix all pre-launch bugs and gaps identified by Opus review before the Homebrew launch — no silent failures, no misleading protocol, a working getting-started example.

**Architecture:** Three independent tracks executed in order: (A) runtime correctness fixes in the execution engine and CLI, (B) dead code and doc cleanup, (C) examples directory and README rewrite. All tracks touch the same repo; each task produces a self-contained commit.

**Tech Stack:** Rust, pest, clap, serde/serde_json, tokio. Tests via `cargo test`. Binary at `target/release/thruline`.

---

## Files modified

| File | Changes |
|---|---|
| `src/runtime/mod.rs` | Tasks 1, 2, 3, 4, 6, 8 |
| `src/cli.rs` | Tasks 5, 6, 7 |
| `src/events.rs` | Tasks 4, 7 |
| `src/runtime/state.rs` | Task 9 |
| `src/validator.rs` | Task 9 |
| `docs/LANGUAGE.md` | Task 9 |
| `examples/sentiment/` | Task 10 (new) |
| `README.md` | Task 11 |

---

## Track A — Runtime correctness

---

### Task 1: Fix resume to reject wrong stage / completed run (BUG-001, TASK-005)

**Files:**
- Modify: `src/runtime/mod.rs:261` (`resume_stage`)
- Test: `src/runtime/mod.rs` inline tests

- [ ] **Step 1: Write failing tests**

In `src/runtime/mod.rs` tests module, add:

```rust
#[test]
fn test_resume_wrong_stage_errors() {
    let mut rt = mk_runtime();
    rt.state.status = RunStatus::AwaitingResume { stage: "a".into() };
    let err = rt.resume_stage("b", vec![]).unwrap_err();
    assert!(err.to_string().contains("awaiting stage 'a'"), "got: {}", err);
}

#[test]
fn test_resume_done_run_errors() {
    let mut rt = mk_runtime();
    rt.state.status = RunStatus::Done;
    let err = rt.resume_stage("a", vec![]).unwrap_err();
    assert!(err.to_string().contains("already done"), "got: {}", err);
}
```

- [ ] **Step 2: Run to confirm they fail**

```
cargo test test_resume_wrong_stage_errors test_resume_done_run_errors 2>&1 | tail -5
```

Expected: FAIL — `resume_stage` currently never checks status.

- [ ] **Step 3: Add validation at the top of `resume_stage`**

In `src/runtime/mod.rs`, add this block immediately after the function signature and before `for (name, value, is_file) in outputs`:

```rust
pub fn resume_stage(
    &mut self,
    stage_name: &str,
    outputs: Vec<(String, String, bool)>,
) -> anyhow::Result<()> {
    match &self.state.status {
        RunStatus::AwaitingResume { stage } if stage == stage_name => {}
        RunStatus::AwaitingResume { stage } => anyhow::bail!(
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
        RunStatus::ParallelAwait { stage, .. } => anyhow::bail!(
            "run '{}' is in parallel await for stage '{}'; parallel resume not yet implemented",
            self.state.run_id, stage
        ),
    }

    for (name, value, is_file) in outputs {
```

- [ ] **Step 4: Run tests**

```
cargo test 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/runtime/mod.rs
git commit -m "fix: resume validates stage name and run status before accepting outputs"
```

---

### Task 2: Warn on parallel syntax until runtime is wired (BUG-002, TASK-006)

**Files:**
- Modify: `src/runtime/mod.rs` (`resume_stage`, route evaluation section ~line 285)

- [ ] **Step 1: Write a test confirming the current silent fallthrough behavior**

```rust
#[test]
fn test_fan_out_route_falls_through_with_no_panic() {
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
                },
                Route {
                    source: RouteSource::FanIn("b".into()),
                    target: RouteTarget { stage: "c".into(), parallel_spec: None },
                },
            ],
        }),
    ];
    let mut rt = Runtime::new(state, items);
    rt.state.status = RunStatus::AwaitingResume { stage: "a".into() };
    rt.resume_stage("a", vec![]).unwrap();
    assert!(matches!(&rt.state.status, RunStatus::AwaitingResume { stage } if stage == "b"));
}
```

- [ ] **Step 2: Run to confirm it already passes**

```
cargo test test_fan_out_route_falls_through 2>&1 | tail -5
```

Expected: PASS (this guards the behavior from regressing).

- [ ] **Step 3: Add warning after the route match in `resume_stage`**

In `src/runtime/mod.rs`, inside the `if let Some((route, next_stage)) = ...` block, immediately before the `ThrulineEvent::RouteTaken` emit:

```rust
        if let Some((route, next_stage)) =
            self.evaluate_routes(stage_name, &routes, &artifacts_snapshot)
        {
            if route.target.parallel_spec.is_some() {
                let limit_str = route.target.parallel_spec.as_ref()
                    .and_then(|s| s.limit)
                    .map(|l| l.to_string())
                    .unwrap_or_default();
                eprintln!(
                    "warning: run {}: route {} -> {}[*{}] is a fan-out — \
                     parallel execution not yet implemented, running as single sequential invocation",
                    self.state.run_id, stage_name, next_stage, limit_str
                );
            }

            let predicate_desc = format_route_source(&route.source);
```

Note: `format_route_source` is added in Task 3. If doing tasks out of order, use `format!("{:?}", route.source)` temporarily.

- [ ] **Step 4: Run tests**

```
cargo test 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/runtime/mod.rs
git commit -m "fix: warn to stderr when fan-out route taken but parallel not yet implemented"
```

---

### Task 3: Fix route_taken.predicate format (BUG-003)

**Files:**
- Modify: `src/runtime/mod.rs` (add `format_route_source`, replace `{:?}` format ~line 292)
- Test: `src/runtime/mod.rs`

- [ ] **Step 1: Write failing test**

```rust
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
```

- [ ] **Step 2: Run to confirm it fails**

```
cargo test test_format_route_source_predicate 2>&1 | tail -5
```

Expected: FAIL.

- [ ] **Step 3: Add `format_route_source` before `impl Runtime`**

```rust
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
```

Replace ~line 292 in `resume_stage`:

```rust
// Before:
let predicate_desc = format!("{:?}", route.source);
// After:
let predicate_desc = format_route_source(&route.source);
```

- [ ] **Step 4: Run tests**

```
cargo test 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/runtime/mod.rs
git commit -m "fix: route_taken.predicate uses stable human-readable format not Rust Debug"
```

---

### Task 4: Fix pipeline_done to include all artifacts (BUG-004)

**Files:**
- Modify: `src/events.rs` — change `PipelineDone.outputs: Vec<String>` to `serde_json::Value`
- Modify: `src/runtime/mod.rs:304` — use `ArtifactStore::to_json()`
- Test: `tests/integration.rs`

- [ ] **Step 1: Write failing integration test**

```rust
#[test]
fn test_pipeline_done_includes_value_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let run_out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output().unwrap();
    let run_id = serde_json::from_str::<serde_json::Value>(
        String::from_utf8_lossy(&run_out.stdout).lines().next().unwrap()
    ).unwrap()["run_id"].as_str().unwrap().to_string();

    // a routes to b on verdict==ok; b has no routes → pipeline_done
    thruline().args(["resume", &run_id, "--stage", "a", "--artifact", "verdict=ok"])
        .output().unwrap();
    let done_out = thruline()
        .args(["resume", &run_id, "--stage", "b"])
        .output().unwrap();
    let done_stdout = String::from_utf8_lossy(&done_out.stdout);

    let done: serde_json::Value = done_stdout.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .find(|v: &serde_json::Value| v["event"] == "pipeline_done")
        .expect("no pipeline_done");

    let outputs = &done["outputs"];
    assert!(outputs.is_object(), "outputs should be an object, got: {}", outputs);
    assert_eq!(outputs["a.verdict"], "ok", "a.verdict missing: {}", outputs);
}
```

- [ ] **Step 2: Run to confirm it fails**

```
cargo test test_pipeline_done_includes_value_artifacts 2>&1 | tail -8
```

Expected: FAIL — `outputs` is `Vec<String>`.

- [ ] **Step 3: Update `PipelineDone` in `src/events.rs`**

```rust
PipelineDone {
    run_id: String,
    ts: DateTime<Utc>,
    outputs: serde_json::Value,
},
```

- [ ] **Step 4: Update the emission in `src/runtime/mod.rs`**

```rust
        } else {
            ThrulineEvent::PipelineDone {
                run_id: self.state.run_id.clone(),
                ts: chrono::Utc::now(),
                outputs: self.state.artifacts.to_json(),
            }
            .emit();
            self.state.status = RunStatus::Done;
        }
```

- [ ] **Step 5: Fix `src/events.rs` test that uses `PipelineDone`**

In `test_all_event_variants_serialize`, change:

```rust
ThrulineEvent::PipelineDone {
    run_id: run_id.clone(), ts,
    outputs: serde_json::json!({"a.result": "done"}),
},
```

- [ ] **Step 6: Run tests**

```
cargo test 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src/events.rs src/runtime/mod.rs tests/integration.rs
git commit -m "fix: pipeline_done.outputs is full artifact map (value + path), not just file keys"
```

---

### Task 5: Move API key check before pipeline_start (BUG-005)

**Files:**
- Modify: `src/cli.rs:300-327` (`cmd_run`)
- Test: `tests/integration.rs`

- [ ] **Step 1: Write failing integration test**

```rust
#[test]
fn test_api_driver_no_key_does_not_emit_pipeline_start() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "api"])
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("THRULINE_DEFAULT_MODEL")
        .output().unwrap();

    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(r#""event":"pipeline_start""#),
        "pipeline_start emitted before key check: {}", stdout
    );
}
```

- [ ] **Step 2: Run to confirm it fails**

```
cargo test test_api_driver_no_key 2>&1 | tail -8
```

Expected: FAIL.

- [ ] **Step 3: Restructure `cmd_run` to resolve driver before emitting events**

In `src/cli.rs`, replace from `let config_model = ...` through the end of `cmd_run`:

```rust
    let config_model = items.iter().find_map(|i| {
        if let TlItem::Config(c) = i { c.model.clone() } else { None }
    });

    let mut runtime = Runtime::new(state, items);
    runtime.check_pipeline_inputs()?;

    match driver_name {
        "stdio" => {
            crate::events::ThrulineEvent::PipelineStart {
                run_id: run_id.clone(),
                ts: chrono::Utc::now(),
                pipeline: pipeline.clone(),
                inputs: serde_json::Value::Null,
            }.emit();
            let driver = crate::driver::stdio::StdioDriver;
            if let Err(e) = runtime.advance(&driver).await {
                crate::events::ThrulineEvent::PipelineError {
                    run_id: run_id.clone(),
                    ts: chrono::Utc::now(),
                    stage: runtime.state.history.last()
                        .cloned().unwrap_or_else(|| "unknown".to_string()),
                    error: e.to_string(),
                }.emit();
                return Err(e);
            }
        }
        "api" => {
            let driver = crate::driver::api::ApiDriver::from_env(config_model)?;
            crate::events::ThrulineEvent::PipelineStart {
                run_id: run_id.clone(),
                ts: chrono::Utc::now(),
                pipeline: pipeline.clone(),
                inputs: serde_json::Value::Null,
            }.emit();
            if let Err(e) = runtime.advance(&driver).await {
                crate::events::ThrulineEvent::PipelineError {
                    run_id: run_id.clone(),
                    ts: chrono::Utc::now(),
                    stage: runtime.state.history.last()
                        .cloned().unwrap_or_else(|| "unknown".to_string()),
                    error: e.to_string(),
                }.emit();
                return Err(e);
            }
        }
        other => anyhow::bail!("unknown driver '{}' \u{2014} use stdio or api", other),
    }

    Ok(())
```

- [ ] **Step 4: Run tests**

```
cargo test 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs
git commit -m "fix: api key validated before pipeline_start; pipeline_error emitted on runtime failure"
```

---

### Task 6: Emit stage_complete and stage_error (TASK-009)

**Files:**
- Modify: `src/runtime/mod.rs` (`resume_stage`)
- Modify: `src/cli.rs` (`cmd_resume`)
- Test: `tests/integration.rs`

- [ ] **Step 1: Write failing integration test**

```rust
#[test]
fn test_resume_emits_stage_complete() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let run_out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output().unwrap();
    let run_id = serde_json::from_str::<serde_json::Value>(
        String::from_utf8_lossy(&run_out.stdout).lines().next().unwrap()
    ).unwrap()["run_id"].as_str().unwrap().to_string();

    let resume_out = thruline()
        .args(["resume", &run_id, "--stage", "a", "--artifact", "verdict=ok"])
        .output().unwrap();
    let resume_stdout = String::from_utf8_lossy(&resume_out.stdout);

    assert!(
        resume_stdout.contains(r#""event":"stage_complete""#),
        "stage_complete not emitted: {}", resume_stdout
    );
    let complete: serde_json::Value = resume_stdout.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .find(|v: &serde_json::Value| v["event"] == "stage_complete")
        .expect("no stage_complete");
    assert_eq!(complete["stage"], "a");
    assert_eq!(complete["outputs"]["verdict"], "ok");
}
```

- [ ] **Step 2: Run to confirm it fails**

```
cargo test test_resume_emits_stage_complete 2>&1 | tail -8
```

Expected: FAIL.

- [ ] **Step 3: Emit `StageComplete` in `resume_stage` after validation**

In `src/runtime/mod.rs`, after the status-validation `match` block and before `for (name, value, is_file) in outputs`:

```rust
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
```

- [ ] **Step 4: Add `stage_error` emission in `cmd_resume` on failure**

In `src/cli.rs`, `cmd_resume`, replace:

```rust
    runtime.resume_stage(stage_name, outputs)?;
```

With:

```rust
    if let Err(e) = runtime.resume_stage(stage_name, outputs) {
        crate::events::ThrulineEvent::StageError {
            run_id: run_id.to_string(),
            ts: chrono::Utc::now(),
            stage: stage_name.to_string(),
            error: e.to_string(),
        }
        .emit();
        return Err(e);
    }
```

Also wrap the second `runtime.advance()` call in `cmd_resume` with `PipelineError` emission:

```rust
    if matches!(runtime.state.status, RunStatus::AwaitingResume { .. }) {
        let driver = crate::driver::stdio::StdioDriver;
        if let Err(e) = runtime.advance(&driver).await {
            crate::events::ThrulineEvent::PipelineError {
                run_id: run_id.to_string(),
                ts: chrono::Utc::now(),
                stage: runtime.state.history.last()
                    .cloned().unwrap_or_else(|| "unknown".to_string()),
                error: e.to_string(),
            }
            .emit();
            return Err(e);
        }
    }
```

- [ ] **Step 5: Run tests**

```
cargo test 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/runtime/mod.rs src/cli.rs
git commit -m "feat: emit stage_complete on resume; stage_error and pipeline_error on failures"
```

---

### Task 7: Add declared outputs to stage_invoke + fix prompt file error (TASK-011, TASK-012)

**Files:**
- Modify: `src/events.rs` — add `OutputDecl`, add `outputs` to `StageInvoke`
- Modify: `src/runtime/mod.rs` — build declared outputs in `advance()`, fix prompt file error
- Test: `tests/integration.rs`

- [ ] **Step 1: Write failing integration test**

```rust
#[test]
fn test_stage_invoke_includes_declared_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "test.line", BASIC_TL);

    let out = thruline()
        .args(["run", tl.to_str().unwrap(), "--driver", "stdio"])
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);

    let invoke_line = stdout.lines()
        .find(|l| l.contains(r#""event":"stage_invoke""#))
        .expect("no stage_invoke");
    let event: serde_json::Value = serde_json::from_str(invoke_line).unwrap();

    // BASIC_TL stage a declares: out: verdict as value
    let outputs = event["outputs"].as_array()
        .expect("outputs field missing from stage_invoke");
    assert_eq!(outputs[0]["name"], "verdict");
    assert_eq!(outputs[0]["kind"], "value");
}
```

- [ ] **Step 2: Run to confirm it fails**

```
cargo test test_stage_invoke_includes_declared_outputs 2>&1 | tail -8
```

Expected: FAIL.

- [ ] **Step 3: Add `OutputDecl` to `src/events.rs`**

After `RunnerSpec`:

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OutputDecl {
    pub name: String,
    pub kind: String,  // "value" or "path"
}
```

Add `outputs` field to `StageInvoke`:

```rust
    StageInvoke {
        run_id: String,
        ts: DateTime<Utc>,
        stage: String,
        runner: RunnerSpec,
        artifacts: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        outputs: Vec<OutputDecl>,
    },
```

- [ ] **Step 4: Build `declared_outputs` in `advance()` in `src/runtime/mod.rs`**

After `runner_spec` is resolved and before `input_artifacts`:

```rust
        let declared_outputs: Vec<crate::events::OutputDecl> = stage.outputs.iter()
            .map(|a| crate::events::OutputDecl {
                name: a.name.clone(),
                kind: match a.kind {
                    ArtifactKind::Path  => "path".to_string(),
                    ArtifactKind::Value => "value".to_string(),
                },
            })
            .collect();
```

Add `outputs: declared_outputs` to the `ThrulineEvent::StageInvoke { ... }` construction.

- [ ] **Step 5: Fix `src/events.rs` test that creates `StageInvoke`**

In `test_stage_invoke_roundtrip`, add `outputs: vec![]` to the struct literal.

- [ ] **Step 6: Fix prompt file error message in `src/runtime/mod.rs`**

In `advance()`, the prompt resolution block:

```rust
            Some(PromptSource::File(p)) => {
                let abs = tl_path.parent().unwrap_or(Path::new(".")).join(p);
                Some(std::fs::read_to_string(&abs).map_err(|_| {
                    anyhow::anyhow!("prompt file not found: {}", abs.display())
                })?)
            }
```

- [ ] **Step 7: Run tests**

```
cargo test 2>&1 | tail -5
```

Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add src/events.rs src/runtime/mod.rs tests/integration.rs
git commit -m "feat: stage_invoke includes declared output specs; fix prompt file error message"
```

---

## Track B — Dead code and doc cleanup

---

### Task 8: Remove dead code and fix false doc claims (TASK-013, BUG-006, TASK-014)

**Files:**
- Modify: `src/runtime/state.rs` — remove `events_path()`
- Modify: `src/validator.rs` — remove `CircularImport` variant
- Modify: `docs/LANGUAGE.md` — fix circular import and parallel keyword claims

- [ ] **Step 1: Remove `events_path()` from `src/runtime/state.rs`**

Delete:

```rust
    pub fn events_path(&self) -> PathBuf {
        self.run_dir().join("events.ndjson")
    }
```

- [ ] **Step 2: Remove `CircularImport` from `src/validator.rs`**

Delete from `ValidationError`:

```rust
    #[error("circular import: {0}")]
    CircularImport(String),
```

- [ ] **Step 3: Fix `docs/LANGUAGE.md` circular import claim**

Find:

> Imports deduplicate by canonical path — importing the same file twice is safe. Circular imports are detected and rejected.

Replace with:

> Imports deduplicate by canonical path — importing the same file twice is safe. Circular references terminate naturally through path deduplication.

- [ ] **Step 4: Remove `parallel` keyword references from `docs/LANGUAGE.md`**

Search for `parallel` in the docs. Remove the sentence "The `parallel` keyword is required on any route targeting `[*N]`." Update any fan-out example that shows `-> b[*3] parallel` to just `-> b[*3]`.

- [ ] **Step 5: Run tests**

```
cargo test 2>&1 | tail -5
```

Expected: all pass; no `CircularImport` dead-code warning.

- [ ] **Step 6: Commit**

```bash
git add src/runtime/state.rs src/validator.rs docs/LANGUAGE.md
git commit -m "chore: remove dead events_path and CircularImport; fix false doc claims"
```

---

## Track C — Examples and README

---

### Task 9: Ship working examples/ directory (TASK-007)

**Files:**
- Create: `examples/sentiment/pipeline.line`
- Create: `examples/sentiment/prompts/analyst.md`
- Create: `examples/sentiment/prompts/writer.md`
- Create: `examples/sentiment/README.md`

- [ ] **Step 1: Create prompt files**

`examples/sentiment/prompts/analyst.md`:
```
You analyze text sentiment.
Respond ONLY with a JSON object: {"sentiment": "positive" or "negative" or "neutral", "score": "1"-"10", "notes": "one sentence"}.
```

`examples/sentiment/prompts/writer.md`:
```
You write brief responses based on sentiment analysis findings.
Respond ONLY with a JSON object: {"message": "your response here"}.
```

- [ ] **Step 2: Create `examples/sentiment/pipeline.line`**

```
config {
  model: claude-haiku-4-5-20251001
}

runner analyst {
  system: file("prompts/analyst.md")
  temperature: 0.3
  max_tokens: 256
}

runner writer {
  system: file("prompts/writer.md")
  max_tokens: 256
}

pipeline sentiment {
  inputs {
    text as value
  }
  start: classify
  routes {
    classify.sentiment == "positive" -> affirm
    classify.sentiment != "positive" -> explain
  }
}

stage classify {
  in:  text      as value
  out: sentiment as value
       score     as value
       notes     as value
  runner: analyst
  prompt: "Analyze the sentiment of the provided text."
}

stage affirm {
  in:  sentiment as value
       notes     as value
  out: message as value
  runner: writer
  prompt: "Write a brief affirming reply based on the sentiment findings."
}

stage explain {
  in:  sentiment as value
       notes     as value
  out: message as value
  runner: writer
  prompt: "Write a brief explanatory reply based on the sentiment findings."
}
```

- [ ] **Step 3: Validate**

```bash
thruline validate examples/sentiment/pipeline.line
```

Expected: `ok`

- [ ] **Step 4: Inspect**

```bash
thruline inspect examples/sentiment/pipeline.line
```

Expected: inputs block with `text as value`, start: classify, two predicate routes, three stages.

- [ ] **Step 5: Create `examples/sentiment/README.md`**

```markdown
# Sentiment Analysis

Classifies text sentiment (positive / negative / neutral) and routes to a tailored response stage.

## Run standalone (Anthropic API)

    ANTHROPIC_API_KEY=sk-... thruline run pipeline.line --driver api --input text="I love Rust!"

## Run in harness mode (stdio driver)

    thruline run pipeline.line --input text="I love Rust!"
    # → emits stage_invoke; drive the agent, then:
    thruline resume <run-id> --stage classify \
      --artifact sentiment=positive \
      --artifact score=9 \
      --artifact notes="Enthusiastic and appreciative tone."
    # continues routing until pipeline_done
```

- [ ] **Step 6: Commit**

```bash
git add examples/
git commit -m "feat: add examples/sentiment — self-contained validated pipeline"
```

---

### Task 10: Fix README getting-started example (TASK-008)

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Verify the current README example fails validation**

Write the current `.line` block from the README to a temp file:

```bash
sed -n '/^```$/,/^```$/p' README.md | head -60 > /tmp/readme-check.line
thruline validate /tmp/readme-check.line 2>&1
```

Expected: error (system prompt file not found, or input doesn't thread through).

- [ ] **Step 2: Replace the .line example with a self-contained, validatable one**

Replace the existing code block in `README.md` with:

```
config {
  model: claude-haiku-4-5-20251001
}

runner analyst {
  system: "You are a concise analyst. Respond ONLY with JSON."
  temperature: 0.3
}

pipeline sentiment {
  inputs {
    text as value
  }
  start: classify
  routes {
    classify.sentiment == "positive" -> affirm
    classify.sentiment != "positive" -> explain
  }
}

stage classify {
  in:  text      as value
  out: sentiment as value
       notes     as value
  runner: analyst
  prompt: "Classify the sentiment of the provided text. Return JSON with sentiment and notes fields."
}

stage affirm {
  in:  notes as value
  out: reply as value
  runner: analyst
  prompt: "Write a brief affirming reply based on the notes."
}

stage explain {
  in:  notes as value
  out: reply as value
  runner: analyst
  prompt: "Write a brief explanatory reply based on the notes."
}
```

- [ ] **Step 3: Fix the run commands shown**

Replace the single run command with:

```bash
# Standalone — calls Anthropic API directly
ANTHROPIC_API_KEY=sk-... thruline run pipeline.line --driver api --input text="I love Rust!"

# Harness mode — harness drives the agents
thruline run pipeline.line --input text="I love Rust!"
# → emits stage_invoke JSON, then drive the agent and resume:
thruline resume <run-id> --stage classify \
  --artifact sentiment=positive --artifact notes="Enthusiastic tone."
```

- [ ] **Step 4: Add pointer to examples/**

After the Install section, add:

```markdown
## Examples

See [`examples/sentiment/`](examples/sentiment/) for a self-contained runnable pipeline.
```

- [ ] **Step 5: Validate the new example**

```bash
cat > /tmp/readme-new.line << 'EOF'
config {
  model: claude-haiku-4-5-20251001
}
runner analyst {
  system: "You are a concise analyst. Respond ONLY with JSON."
  temperature: 0.3
}
pipeline sentiment {
  inputs { text as value }
  start: classify
  routes {
    classify.sentiment == "positive" -> affirm
    classify.sentiment != "positive" -> explain
  }
}
stage classify {
  in:  text as value
  out: sentiment as value
       notes     as value
  runner: analyst
  prompt: "Classify the sentiment of the text."
}
stage affirm  { in: notes as value  out: reply as value  runner: analyst  prompt: "Affirm." }
stage explain { in: notes as value  out: reply as value  runner: analyst  prompt: "Explain." }
EOF
thruline validate /tmp/readme-new.line
```

Expected: `ok`

- [ ] **Step 6: Commit**

```bash
git add README.md
git commit -m "fix: README example validates, threads inputs through stages, shows both drivers"
```

---

## Final verification

- [ ] **Run full test suite**

```
cargo test 2>&1
```

Expected: all tests pass (was 77 before this plan).

- [ ] **Check for eliminated warnings**

```bash
cargo build 2>&1 | grep "warning.*unused\|warning.*never"
```

`CircularImport` and `events_path` warnings should be gone.

- [ ] **Validate examples**

```bash
thruline validate examples/sentiment/pipeline.line
thruline inspect examples/sentiment/pipeline.line
```

Both succeed with no errors.

- [ ] **Push and tag v0.1.1**

```bash
git push origin main
git tag v0.1.1
git push origin v0.1.1
```

- [ ] **Update Homebrew formula to v0.1.1**

```bash
SHA=$(curl -sL https://github.com/pufferhaus/thruline/archive/refs/tags/v0.1.1.tar.gz | shasum -a 256 | cut -d' ' -f1)
# Edit pufferhaus/homebrew-tap Formula/thruline.rb:
#   url  → https://github.com/pufferhaus/thruline/archive/refs/tags/v0.1.1.tar.gz
#   sha256 → $SHA
```
