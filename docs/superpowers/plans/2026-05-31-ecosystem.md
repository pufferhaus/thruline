# Ecosystem Expansion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Three ecosystem additions: (1) library mode — expose `thruline-core` as an embeddable crate so Rust services can drive pipelines without shelling out; (2) `thruline serve` — read-only local web UI over `~/.thruline/runs/`; (3) example gallery — 3 new examples covering fan-out, multi-runner, and real-world patterns.

**Architecture:** Library mode: add `src/lib.rs` re-exporting `Runtime`, `Driver`, `RunState`, `ThrulineEvent`, and `ArtifactStore`. Serve: new `thruline serve` subcommand using `axum` + `tokio`; reads existing state files, serves JSON API + simple HTML viewer. Examples: three new self-contained directories under `examples/`.

**Tech Stack:** Library mode — no new deps. Serve — `axum` (add to Cargo.toml). Examples — `.line` + markdown only.

---

## Files

**Library mode:**
- Create: `src/lib.rs`
- Modify: `Cargo.toml` — expose as both `lib` and `bin`

**Serve:**
- Modify: `Cargo.toml` — add `axum`, `tower`
- Create: `src/serve.rs`
- Modify: `src/cli.rs` — `Serve` subcommand + `cmd_serve()`
- Modify: `src/main.rs` — `mod serve;`

**Examples:**
- Create: `examples/pr-triage/`
- Create: `examples/doc-gen/`
- Create: `examples/multi-reviewer/`

**Docs:**
- Modify: `README.md` — update examples table, add library usage section
- Modify: `docs/LANGUAGE.md` — library mode section

---

### Task 1: Library mode — `src/lib.rs` (embeddable crate)

**Files:**
- Create: `src/lib.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add `[lib]` section to `Cargo.toml`**

```toml
[lib]
name = "thruline"
path = "src/lib.rs"

[[bin]]
name = "thruline"
path = "src/main.rs"
```

- [ ] **Step 2: Create `src/lib.rs` re-exporting the public surface**

```rust
//! Thruline — deterministic agent pipeline runtime.
//!
//! Embed thruline in a Rust application instead of shelling out:
//!
//! ```rust,no_run
//! use thruline::Runtime;
//! use thruline::driver::StdioDriver;
//! use thruline::state::RunState;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let items = thruline::parser::parse_file("pipeline.line")?;
//! let state = RunState::new("run-1".into(), "my-pipeline".into(), "pipeline.line".into());
//! let mut runtime = Runtime::new(state, items);
//! let driver = StdioDriver;
//! runtime.advance(&driver).await?;
//! # Ok(())
//! # }
//! ```

pub mod ast;
pub mod driver;
pub mod events;
pub mod parser;
pub mod runtime;
pub mod validator;

pub use runtime::Runtime;
pub use runtime::AdvanceOutcome;
pub use runtime::state::RunState;
pub use runtime::artifact::ArtifactStore;
pub use events::ThrulineEvent;
```

- [ ] **Step 3: Ensure all referenced modules are `pub` where needed**

Check that `src/parser/mod.rs` exposes `pub fn parse_file(path: &Path) -> anyhow::Result<Vec<TlItem>>`. If `parse_file` doesn't exist, add it:

```rust
pub fn parse_file(path: &std::path::Path) -> anyhow::Result<Vec<crate::ast::TlItem>> {
    let src = std::fs::read_to_string(path)
        .map_err(|_| anyhow::anyhow!("file not found: {}", path.display()))?;
    parse_str(&src)
}
```

- [ ] **Step 4: Compile both targets**

```bash
cargo build 2>&1 | grep '^error'
cargo build --lib 2>&1 | grep '^error'
```
Expected: no errors.

- [ ] **Step 5: Add a library integration test**

Create `tests/lib_usage.rs`:

```rust
use thruline::{Runtime, runtime::state::RunState};
use thruline::ast::TlItem;

#[test]
fn test_runtime_constructable_from_lib() {
    let state = RunState::new("r".into(), "p".into(), "/tmp/test.line".into());
    let items: Vec<TlItem> = vec![];
    let runtime = Runtime::new(state, items);
    assert_eq!(runtime.state.run_id, "r");
}
```

```bash
cargo test 2>&1 | tail -3
```
Expected: all pass.

- [ ] **Step 6: Add library usage section to `docs/LANGUAGE.md`**

```markdown
## Library Usage

Add to `Cargo.toml`:
```toml
[dependencies]
thruline = { git = "https://github.com/pufferhaus/thruline" }
```

Basic usage:
```rust
use thruline::{Runtime, runtime::state::RunState};
use thruline::driver::stdio::StdioDriver;

let items = thruline::parser::parse_file("pipeline.line".as_ref())?;
let state = RunState::new(run_id, pipeline_name, "pipeline.line".into());
let mut runtime = Runtime::new(state, items);
runtime.check_pipeline_inputs()?;
let driver = StdioDriver;
runtime.advance(&driver).await?;
```

Implement the `Driver` trait to plug in your own agent backend.
```

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs Cargo.toml tests/lib_usage.rs docs/LANGUAGE.md
git commit -m "feat: expose thruline as an embeddable library crate"
```

---

### Task 2: `thruline serve` — local run inspection UI

**Files:**
- Modify: `Cargo.toml` — add `axum`, `tower-http`
- Create: `src/serve.rs`
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Add `axum` and `tower-http` to `Cargo.toml`**

```toml
axum = "0.7"
tower-http = { version = "0.5", features = ["fs", "cors"] }
```

- [ ] **Step 2: Add `Serve` subcommand to `src/cli.rs`**

```rust
/// Serve a read-only web UI for run inspection
Serve {
    /// Port to listen on
    #[arg(long, default_value = "7371")]
    port: u16,
},
```

In `run()`:
```rust
Commands::Serve { port } => cmd_serve(port).await,
```

Add:
```rust
pub async fn cmd_serve(port: u16) -> anyhow::Result<()> {
    crate::serve::run_server(port).await
}
```

- [ ] **Step 3: Create `src/serve.rs`**

```rust
use axum::{routing::get, Router, Json, extract::Path};
use crate::runtime::state::{list_runs, RunState};

pub async fn run_server(port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(handler_index))
        .route("/api/runs", get(handler_list_runs))
        .route("/api/runs/:id", get(handler_get_run));

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("thruline serve: http://{}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handler_index() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("serve_index.html"))
}

async fn handler_list_runs() -> Json<Vec<serde_json::Value>> {
    let runs = list_runs().unwrap_or_default();
    Json(runs.into_iter().map(|r| serde_json::to_value(r).unwrap()).collect())
}

async fn handler_get_run(Path(id): Path<String>) -> Result<Json<RunState>, axum::http::StatusCode> {
    RunState::load(&id)
        .map(Json)
        .map_err(|_| axum::http::StatusCode::NOT_FOUND)
}
```

- [ ] **Step 4: Create `src/serve_index.html`** (embedded static UI)

```html
<!DOCTYPE html>
<html>
<head>
  <title>Thruline Runs</title>
  <style>
    body { font-family: monospace; max-width: 900px; margin: 2em auto; padding: 0 1em; }
    h1 { font-size: 1.2em; }
    .run { border: 1px solid #ddd; padding: 1em; margin-bottom: 1em; border-radius: 4px; }
    .status-done { color: green; }
    .status-failed { color: red; }
    .status-running, .status-awaiting { color: orange; }
    pre { background: #f5f5f5; padding: 0.5em; overflow-x: auto; font-size: 0.85em; }
  </style>
</head>
<body>
  <h1>Thruline Runs</h1>
  <div id="runs">Loading...</div>
  <script>
    async function load() {
      const runs = await fetch('/api/runs').then(r => r.json());
      const el = document.getElementById('runs');
      if (!runs.length) { el.textContent = 'No runs yet.'; return; }
      el.innerHTML = runs.map(r => `
        <div class="run">
          <strong>${r.run_id}</strong> — <em>${r.pipeline}</em>
          <span class="status-${statusClass(r.status)}">[${statusText(r.status)}]</span>
          <span style="float:right;color:#888">${r.started}</span>
          <details><summary>Artifacts</summary><pre>${JSON.stringify(r.artifacts, null, 2)}</pre></details>
        </div>
      `).join('');
    }
    function statusClass(s) {
      if (typeof s === 'string') return s.toLowerCase();
      if (s.AwaitingResume) return 'awaiting';
      if (s.Failed) return 'failed';
      return 'running';
    }
    function statusText(s) {
      if (s === 'Done') return 'done';
      if (s === 'Running') return 'running';
      if (s.AwaitingResume) return `awaiting: ${s.AwaitingResume.stage}`;
      if (s.Failed) return `failed: ${s.Failed}`;
      return JSON.stringify(s);
    }
    load();
    setInterval(load, 3000);
  </script>
</body>
</html>
```

- [ ] **Step 5: Add `mod serve;` to `src/main.rs`**

```rust
mod serve;
```

- [ ] **Step 6: Compile check**

```bash
cargo build 2>&1 | grep '^error' | head -10
```
Expected: no errors.

- [ ] **Step 7: Smoke test**

```bash
cargo run -- serve --port 7371 &
sleep 1
curl -s http://localhost:7371/api/runs | head -20
kill %1
```
Expected: JSON array (empty or list of runs).

- [ ] **Step 8: Add serve usage to README**

In `README.md` CLI section, add:
```
thruline serve [--port 7371]       # Local run inspection UI at http://localhost:7371
```

- [ ] **Step 9: Commit**

```bash
git add src/serve.rs src/serve_index.html src/cli.rs src/main.rs Cargo.toml README.md
git commit -m "feat: thruline serve — local web UI for run inspection"
```

---

### Task 3: Example gallery — 3 new examples

**Files:**
- Create: `examples/pr-triage/`
- Create: `examples/doc-gen/`
- Create: `examples/multi-reviewer/`
- Modify: `README.md`

- [ ] **Step 1: Create `examples/pr-triage/`**

`examples/pr-triage/triage.line`:
```
// PR triage: classify a PR description, route to different handling.
//
// Inputs: description as value
// Run: thruline run triage.line --driver api --input description="Add retry logic to payment processor"

config {
  model: claude-sonnet-4-6
}

runner classifier {
  system: "You classify pull requests. Respond with JSON only."
  temperature: 0.1
}

runner responder {
  system: "You write clear, helpful PR feedback. Be concise."
}

thruline pr_triage {
  inputs {
    description as value
  }
  start: classify
  routes {
    classify.category == "bug_fix"      -> ack_bug
    classify.category == "feature"      -> ack_feature
    classify.category == "refactor"     -> ack_refactor
    classify -> ack_other
  }
}

stage classify {
  in:  description as value
  out: category as value in ["bug_fix","feature","refactor","docs","other"]
       summary  as value
  runner: classifier
  prompt: "Classify this PR. Return JSON: {\"category\": \"bug_fix|feature|refactor|docs|other\", \"summary\": \"one sentence\"}."
}

stage ack_bug {
  in:  summary as value
  out: response as value
  runner: responder
  prompt: "Write a brief thank-you for this bug fix PR, noting it will be reviewed for correctness and test coverage."
}

stage ack_feature {
  in:  summary as value
  out: response as value
  runner: responder
  prompt: "Write a brief acknowledgment for this feature PR, noting it will be reviewed for design and compatibility."
}

stage ack_refactor {
  in:  summary as value
  out: response as value
  runner: responder
  prompt: "Write a brief acknowledgment for this refactor PR, noting it will be reviewed for behavior preservation."
}

stage ack_other {
  in:  summary as value
  out: response as value
  runner: responder
  prompt: "Write a brief acknowledgment for this PR."
}
```

`examples/pr-triage/README.md`:
```markdown
# PR Triage

Classifies a PR description and routes to category-specific acknowledgment.

**Stages:** `classify` → (one of) `ack_bug` / `ack_feature` / `ack_refactor` / `ack_other`

## Run

    ANTHROPIC_API_KEY=sk-... thruline run triage.line --driver api \
      --input description="Add retry logic to the payment processor on timeout"
```

- [ ] **Step 2: Create `examples/doc-gen/`**

`examples/doc-gen/doc.line`:
```
// Documentation generator: draft docs, review, revise until approved.
//
// Inputs: code as path, context? as value
// Run: thruline run doc.line --driver api --input code=file:///path/to/module.rs

config {
  model: claude-sonnet-4-6
}

runner writer {
  system: "You write clear, accurate technical documentation. Respond with JSON."
  temperature: 0.4
}

runner reviewer {
  system: "You review technical documentation for accuracy and completeness. Respond with JSON."
  temperature: 0.1
}

thruline doc_gen {
  inputs {
    code    as path
    context? as value
  }
  start: draft
  routes {
    review.verdict == "approved" -> finalize
    review.verdict != "approved" -> draft [max:4]
    finalize -> done
  }
}

stage draft {
  in:  code    as path
       context? as value
       feedback? as value
  out: draft as value
  runner: writer
  prompt: "Write API documentation for this code. If feedback is provided, incorporate it. Return JSON: {\"draft\": \"markdown string\"}."
}

stage review {
  in:  draft as value
  out: verdict  as value in ["approved","rejected"]
       feedback as value
  runner: reviewer
  prompt: "Review this documentation for accuracy and completeness. Return JSON: {\"verdict\": \"approved|rejected\", \"feedback\": \"improvement notes or empty string\"}."
}

stage finalize {
  in:  draft as value
  out: docs as value
  runner: writer
  prompt: "Lightly polish this documentation for final output. Return JSON: {\"docs\": \"final markdown string\"}."
}

stage done {
  in:  docs as value
  out: result as value
  prompt: "Output the final documentation as-is."
}
```

`examples/doc-gen/README.md`:
```markdown
# Documentation Generator

Drafts documentation for a code file, reviews it, and revises up to 4 times.

**Stages:** `draft` ↔ `review` (loop, max 4) → `finalize` → `done`

## Run

    ANTHROPIC_API_KEY=sk-... thruline run doc.line --driver api \
      --input code=file:///path/to/module.rs
```

- [ ] **Step 3: Create `examples/multi-reviewer/`**

`examples/multi-reviewer/review.line`:
```
// Multi-perspective code review using run blocks.
//
// Inputs: code as path
// Run: thruline run review.line --driver api --input code=file:///path/to/file.rs

config {
  model: claude-sonnet-4-6
}

runner security {
  system: "You are a security engineer. Review code for vulnerabilities. Respond with JSON."
  temperature: 0.1
}

runner performance {
  system: "You are a performance engineer. Review code for efficiency issues. Respond with JSON."
  temperature: 0.1
}

runner maintainability {
  system: "You are a senior engineer focused on code quality. Review for maintainability. Respond with JSON."
  temperature: 0.2
}

runner synthesizer {
  system: "You synthesize multiple code review perspectives into a final report."
}

thruline multi_review {
  inputs {
    code as path
  }
  start: review
  routes {
    review -> synthesize
  }
}

stage review {
  in:  code as path
  run security_check {
    runner: security
    prompt: "Check for security vulnerabilities. Return JSON: {\"issues\": [...], \"severity\": \"low|medium|high\"}."
    out: security_issues  as value
         security_severity as value in ["low","medium","high"]
  }
  run performance_check {
    runner: performance
    prompt: "Check for performance issues. Return JSON: {\"issues\": [...], \"severity\": \"low|medium|high\"}."
    out: perf_issues   as value
         perf_severity as value in ["low","medium","high"]
  }
  run quality_check {
    runner: maintainability
    prompt: "Check for maintainability issues. Return JSON: {\"issues\": [...], \"severity\": \"low|medium|high\"}."
    out: quality_issues   as value
         quality_severity as value in ["low","medium","high"]
  }
}

stage synthesize {
  in:  security_issues  as value
       perf_issues      as value
       quality_issues   as value
  out: report as value
  runner: synthesizer
  prompt: "Synthesize these three code review perspectives into a final report with prioritized action items."
}
```

`examples/multi-reviewer/README.md`:
```markdown
# Multi-Perspective Code Review

Runs three parallel reviewers (security, performance, maintainability) via run blocks, then synthesizes into one report.

**Stages:** `review` (3 parallel run blocks) → `synthesize`

## Run

    ANTHROPIC_API_KEY=sk-... thruline run review.line --driver api \
      --input code=file:///path/to/file.rs

## Harness mode

    thruline run review.line --input code=file:///path/to/file.rs
    # → emits parallel_start + 3x stage_invoke with run field
    thruline resume <id> --stage review --run security_check \
      --artifact security_issues="No issues found." --artifact security_severity=low
    thruline resume <id> --stage review --run performance_check \
      --artifact perf_issues="Consider caching." --artifact perf_severity=medium
    thruline resume <id> --stage review --run quality_check \
      --artifact quality_issues="Functions too long." --artifact quality_severity=medium
    # → all runs done, emits parallel_done, then stage_invoke for synthesize
```

- [ ] **Step 4: Validate all three examples**

```bash
cargo run -- validate examples/pr-triage/triage.line
cargo run -- validate examples/doc-gen/doc.line
cargo run -- validate examples/multi-reviewer/review.line
```
Expected: `ok` for each.

- [ ] **Step 5: Update README examples table**

```markdown
| [`examples/sentiment/`](examples/sentiment/) | Predicate routing, two runners, pipeline inputs |
| [`examples/code-review/`](examples/code-review/) | Retry loop, pass/fail routing |
| [`examples/pr-triage/`](examples/pr-triage/) | Multi-branch routing, value constraints |
| [`examples/doc-gen/`](examples/doc-gen/) | Retry loop with max visits, two-phase review |
| [`examples/multi-reviewer/`](examples/multi-reviewer/) | Run blocks (parallel), multi-runner |
```

- [ ] **Step 6: Commit and push**

```bash
git add examples/pr-triage/ examples/doc-gen/ examples/multi-reviewer/ README.md
git commit -m "feat: example gallery — pr-triage, doc-gen, multi-reviewer (A1 examples)"
git push origin main
```
