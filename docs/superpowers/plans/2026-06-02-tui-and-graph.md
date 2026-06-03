# TUI + Graph Visualizer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `thruline -i` (interactive TUI: file browser, pipeline flowchart, run launcher, live event log) and `thruline graph <file>` (ASCII flowchart to stdout).

**Architecture:** ratatui + crossterm for TUI. A shared `src/tui/visualizer.rs` builds a directed graph from the AST and renders ASCII art — used by both the TUI middle pane and the `graph` CLI command. Runs are launched by spawning `thruline run ...` as child processes, streaming NDJSON via tokio channels. The existing `thruline` lib (Runtime, RunState, load_items) is unchanged.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28 (with event-stream feature), tokio (existing), futures 0.3 (for EventStream).

---

## File Map

| File | Status | Responsibility |
|---|---|---|
| `Cargo.toml` | modify | add ratatui, crossterm, futures deps |
| `src/main.rs` | modify | add `mod tui;` |
| `src/cli.rs` | modify | add `-i` flag, `Graph` subcommand, call cmd_tui/cmd_graph |
| `src/tui/mod.rs` | create | entry point: `cmd_tui()`, main event loop |
| `src/tui/app.rs` | create | `App` state: file browser, run list, event logs, modal, channel |
| `src/tui/ui.rs` | create | ratatui render: three panes + modal overlay |
| `src/tui/visualizer.rs` | create | graph build from AST + ASCII render (shared with cmd_graph) |
| `src/tui/runner.rs` | create | spawn subprocess, stream NDJSON lines to channel |
| `.gitignore` | modify | add `.superpowers/` |

---

## Task 1: Dependencies, module scaffold, gitignore

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs`
- Modify: `src/cli.rs`
- Modify: `.gitignore`
- Create: `src/tui/mod.rs`
- Create: `src/tui/app.rs`
- Create: `src/tui/ui.rs`
- Create: `src/tui/visualizer.rs`
- Create: `src/tui/runner.rs`

- [ ] **Step 1: Add deps to Cargo.toml**

In `[dependencies]` section of `Cargo.toml`, add after the existing `async-trait` line:

```toml
ratatui = { version = "0.29", features = ["crossterm"] }
crossterm = { version = "0.28", features = ["event-stream"] }
futures = "0.3"
```

- [ ] **Step 2: Add `mod tui;` to main.rs**

Replace the contents of `src/main.rs` with:

```rust
mod ast;
mod cli;
mod driver;
mod events;
mod lsp;
mod parser;
mod runtime;
mod serve;
mod tui;
mod validator;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::run().await
}
```

- [ ] **Step 3: Create stub tui module files**

Create `src/tui/mod.rs`:
```rust
pub mod app;
pub mod runner;
pub mod ui;
pub mod visualizer;

pub async fn cmd_tui() -> anyhow::Result<()> {
    todo!("TUI not yet implemented")
}
```

Create `src/tui/app.rs`:
```rust
pub struct App;
```

Create `src/tui/ui.rs`:
```rust
pub fn render(_f: &mut ratatui::Frame, _app: &super::app::App) {}
```

Create `src/tui/visualizer.rs`:
```rust
pub struct Graph {
    pub stages: Vec<String>,
    pub edges: Vec<Edge>,
    pub start: String,
}

pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: String,
    pub max_visits: Option<u32>,
}

pub fn build_graph(
    _items: &[crate::ast::TlItem],
    _pipeline_name: Option<&str>,
) -> anyhow::Result<Graph> {
    todo!()
}

pub fn render_graph(_graph: &Graph) -> Vec<String> {
    todo!()
}
```

Create `src/tui/runner.rs`:
```rust
pub async fn spawn_run(
    _file: &std::path::Path,
    _driver: &str,
    _inputs: &[String],
    _pipeline: Option<&str>,
    _tx: tokio::sync::mpsc::Sender<String>,
) -> anyhow::Result<()> {
    todo!()
}
```

- [ ] **Step 4: Add Graph subcommand and -i flag to cli.rs**

Replace the `Cli` struct at the top of `src/cli.rs` with:

```rust
#[derive(Parser)]
#[command(name = "thruline", about = "Deterministic agent pipelines")]
pub struct Cli {
    /// Launch interactive TUI
    #[arg(short = 'i', long = "interactive")]
    pub interactive: bool,
    #[command(subcommand)]
    pub command: Option<Commands>,
}
```

Add `Graph` to the `Commands` enum (after the existing `Resume` variant):

```rust
    /// Render the pipeline flow as ASCII art
    Graph {
        file: PathBuf,
        /// Select pipeline when file declares multiple
        #[arg(long)]
        pipeline: Option<String>,
    },
```

Replace the `run()` function body in `src/cli.rs`:

```rust
pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.interactive {
        return crate::tui::cmd_tui().await;
    }
    match cli.command.ok_or_else(|| anyhow::anyhow!("no command given — use --help"))? {
        Commands::Validate { file }  => cmd_validate(&file),
        Commands::Inspect  { file }  => cmd_inspect(&file),
        Commands::Runs               => cmd_runs(),
        Commands::Status { run_id }  => cmd_status(&run_id),
        Commands::Serve { port }     => cmd_serve(port).await,
        Commands::Lsp                => cmd_lsp().await,
        Commands::Graph { file, pipeline } => cmd_graph(&file, pipeline.as_deref()),
        Commands::Run { file, pipeline, driver, inputs, mock_file } => cmd_run(&file, pipeline.as_deref(), &driver, &inputs, mock_file.as_deref()).await,
        Commands::Resume { run_id, stage, run, artifacts } => cmd_resume(&run_id, &stage, run.as_deref(), &artifacts).await,
    }
}
```

Add stub `cmd_graph` function at the bottom of `src/cli.rs` (before the closing):

```rust
fn cmd_graph(file: &Path, pipeline: Option<&str>) -> anyhow::Result<()> {
    let items = load_items(file)?;
    let graph = crate::tui::visualizer::build_graph(&items, pipeline)?;
    for line in crate::tui::visualizer::render_graph(&graph) {
        println!("{}", line);
    }
    Ok(())
}
```

- [ ] **Step 5: Update .gitignore**

Replace contents of `.gitignore` with:
```
/target
.superpowers/
```

- [ ] **Step 6: Verify it compiles**

```bash
cargo build 2>&1 | head -40
```

Expected: build succeeds (todo! panics are runtime, not compile errors). Any compile errors must be fixed before continuing.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/cli.rs src/tui/ .gitignore
git commit -m "feat: scaffold tui module + graph subcommand + -i flag"
```

---

## Task 2: Visualizer — graph build from AST

**Files:**
- Modify: `src/tui/visualizer.rs`
- Test: `src/tui/visualizer.rs` (inline `#[cfg(test)]`)

The visualizer needs to extract stages and routes from a `PipelineDecl` and build a `Graph`. Note: in `src/ast.rs`, pipelines are `TlItem::Pipeline(PipelineDecl)` even though the DSL keyword is `thruline`.

- [ ] **Step 1: Write failing tests**

Replace `src/tui/visualizer.rs` with:

```rust
use crate::ast::{TlItem, PipelineDecl, StageDecl, Route, RouteSource, RouteTarget, ArtifactKind, ArtifactDecl, CompareOp};

#[derive(Debug, Clone)]
pub struct Graph {
    pub stages: Vec<String>,
    pub edges: Vec<Edge>,
    pub start: String,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: String,       // empty string for unconditional
    pub max_visits: Option<u32>,
    pub is_back_edge: bool,  // true if this edge creates a cycle
}

pub fn build_graph(
    items: &[TlItem],
    pipeline_name: Option<&str>,
) -> anyhow::Result<Graph> {
    todo!()
}

pub fn render_graph(graph: &Graph) -> Vec<String> {
    todo!()
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn edge_label(source: &RouteSource) -> String {
    match source {
        RouteSource::Stage(_) | RouteSource::FanIn(_) => String::new(),
        RouteSource::Predicate { artifact, op, value, .. } => {
            let op_str = match op { CompareOp::Eq => "==", CompareOp::Ne => "!=" };
            format!("{} {} \"{}\"", artifact, op_str, value)
        }
    }
}

fn source_stage(source: &RouteSource) -> &str {
    match source {
        RouteSource::Stage(s) | RouteSource::FanIn(s) => s,
        RouteSource::Predicate { stage, .. } => stage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{InputDecl, PromptSource};

    fn make_items(pipeline: PipelineDecl, stages: Vec<StageDecl>) -> Vec<TlItem> {
        let mut items: Vec<TlItem> = stages.into_iter().map(TlItem::Stage).collect();
        items.push(TlItem::Pipeline(pipeline));
        items
    }

    fn simple_stage(name: &str) -> StageDecl {
        StageDecl {
            name: name.to_string(),
            inputs: vec![],
            outputs: vec![],
            runner: None,
            prompt: None,
            runs: vec![],
        }
    }

    fn unconditional_route(from: &str, to: &str) -> Route {
        Route {
            source: RouteSource::Stage(from.to_string()),
            target: RouteTarget { stage: to.to_string(), parallel_spec: None },
            max_visits: None,
        }
    }

    fn predicate_route(stage: &str, artifact: &str, op: CompareOp, value: &str, to: &str) -> Route {
        Route {
            source: RouteSource::Predicate {
                stage: stage.to_string(),
                artifact: artifact.to_string(),
                op,
                value: value.to_string(),
            },
            target: RouteTarget { stage: to.to_string(), parallel_spec: None },
            max_visits: None,
        }
    }

    #[test]
    fn test_build_graph_linear() {
        let p = PipelineDecl {
            name: "p".to_string(),
            inputs: vec![],
            start: "a".to_string(),
            routes: vec![unconditional_route("a", "b")],
        };
        let items = make_items(p, vec![simple_stage("a"), simple_stage("b")]);
        let graph = build_graph(&items, None).unwrap();
        assert_eq!(graph.start, "a");
        assert!(graph.stages.contains(&"a".to_string()));
        assert!(graph.stages.contains(&"b".to_string()));
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from, "a");
        assert_eq!(graph.edges[0].to, "b");
        assert!(!graph.edges[0].is_back_edge);
    }

    #[test]
    fn test_build_graph_loop_marked() {
        // a -> b -> a (loop)
        let p = PipelineDecl {
            name: "p".to_string(),
            inputs: vec![],
            start: "a".to_string(),
            routes: vec![
                unconditional_route("a", "b"),
                unconditional_route("b", "a"),
            ],
        };
        let items = make_items(p, vec![simple_stage("a"), simple_stage("b")]);
        let graph = build_graph(&items, None).unwrap();
        let back = graph.edges.iter().find(|e| e.to == "a").unwrap();
        assert!(back.is_back_edge, "b->a should be a back edge");
    }

    #[test]
    fn test_build_graph_predicate_label() {
        let p = PipelineDecl {
            name: "p".to_string(),
            inputs: vec![],
            start: "review".to_string(),
            routes: vec![
                predicate_route("review", "verdict", CompareOp::Eq, "approved", "done"),
                predicate_route("review", "verdict", CompareOp::Ne, "approved", "revise"),
            ],
        };
        let items = make_items(p, vec![simple_stage("review"), simple_stage("done"), simple_stage("revise")]);
        let graph = build_graph(&items, None).unwrap();
        let approved_edge = graph.edges.iter().find(|e| e.to == "done").unwrap();
        assert_eq!(approved_edge.label, r#"verdict == "approved""#);
    }

    #[test]
    fn test_build_graph_selects_by_name() {
        let p1 = PipelineDecl { name: "first".to_string(), inputs: vec![], start: "a".to_string(), routes: vec![] };
        let p2 = PipelineDecl { name: "second".to_string(), inputs: vec![], start: "b".to_string(), routes: vec![] };
        let items = vec![
            TlItem::Stage(simple_stage("a")),
            TlItem::Stage(simple_stage("b")),
            TlItem::Pipeline(p1),
            TlItem::Pipeline(p2),
        ];
        let g = build_graph(&items, Some("second")).unwrap();
        assert_eq!(g.start, "b");
    }

    #[test]
    fn test_build_graph_error_no_pipeline() {
        let items = vec![TlItem::Stage(simple_stage("a"))];
        assert!(build_graph(&items, None).is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test tui::visualizer 2>&1 | tail -20
```

Expected: compile error or `not yet implemented` panics. No green tests yet.

- [ ] **Step 3: Implement `build_graph`**

Replace the `todo!()` in `build_graph` with:

```rust
pub fn build_graph(
    items: &[TlItem],
    pipeline_name: Option<&str>,
) -> anyhow::Result<Graph> {
    let pipeline = items.iter()
        .filter_map(|i| if let TlItem::Pipeline(p) = i { Some(p) } else { None })
        .find(|p| pipeline_name.map_or(true, |name| p.name == name))
        .ok_or_else(|| anyhow::anyhow!("no pipeline found"))?;

    // Collect all stage names in insertion order (start first, then route sources/targets)
    let mut stages: Vec<String> = Vec::new();
    let mut seen_stages = std::collections::HashSet::new();

    let add_stage = |name: &str, stages: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
        if seen.insert(name.to_string()) {
            stages.push(name.to_string());
        }
    };

    add_stage(&pipeline.start, &mut stages, &mut seen_stages);
    for route in &pipeline.routes {
        add_stage(source_stage(&route.source), &mut stages, &mut seen_stages);
        add_stage(&route.target.stage, &mut stages, &mut seen_stages);
    }

    // Detect back edges via DFS from start
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut in_stack: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut back_edges: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

    fn dfs(
        node: &str,
        pipeline: &PipelineDecl,
        visited: &mut std::collections::HashSet<String>,
        in_stack: &mut std::collections::HashSet<String>,
        back_edges: &mut std::collections::HashSet<(String, String)>,
    ) {
        if in_stack.contains(node) { return; }
        if visited.contains(node) { return; }
        visited.insert(node.to_string());
        in_stack.insert(node.to_string());
        for route in &pipeline.routes {
            if source_stage(&route.source) == node {
                let to = &route.target.stage;
                if in_stack.contains(to.as_str()) {
                    back_edges.insert((node.to_string(), to.clone()));
                } else {
                    dfs(to, pipeline, visited, in_stack, back_edges);
                }
            }
        }
        in_stack.remove(node);
    }

    dfs(&pipeline.start, pipeline, &mut visited, &mut in_stack, &mut back_edges);

    let edges = pipeline.routes.iter().map(|route| {
        let from = source_stage(&route.source).to_string();
        let to = route.target.stage.clone();
        let is_back_edge = back_edges.contains(&(from.clone(), to.clone()));
        Edge {
            from: from.clone(),
            to,
            label: edge_label(&route.source),
            max_visits: route.max_visits,
            is_back_edge,
        }
    }).collect();

    Ok(Graph { stages, edges, start: pipeline.start.clone() })
}
```

- [ ] **Step 4: Run tests again**

```bash
cargo test tui::visualizer::tests 2>&1 | tail -30
```

Expected: all 5 tests pass. If any fail, fix before continuing.

- [ ] **Step 5: Commit**

```bash
git add src/tui/visualizer.rs
git commit -m "feat: visualizer graph build from AST with back-edge detection"
```

---

## Task 3: Visualizer — ASCII render

**Files:**
- Modify: `src/tui/visualizer.rs`

The renderer does a top-down layout: start stage at top, follow forward edges depth-first. Back edges are rendered as `(loop ↑)` leaf nodes.

- [ ] **Step 1: Write failing render tests**

Add to the `#[cfg(test)]` block in `src/tui/visualizer.rs`:

```rust
    #[test]
    fn test_render_single_stage() {
        let graph = Graph {
            start: "a".to_string(),
            stages: vec!["a".to_string()],
            edges: vec![],
        };
        let lines = render_graph(&graph);
        let joined = lines.join("\n");
        assert!(joined.contains("[a]"), "got: {}", joined);
    }

    #[test]
    fn test_render_linear_two_stages() {
        let graph = Graph {
            start: "a".to_string(),
            stages: vec!["a".to_string(), "b".to_string()],
            edges: vec![Edge { from: "a".to_string(), to: "b".to_string(), label: String::new(), max_visits: None, is_back_edge: false }],
        };
        let lines = render_graph(&graph);
        let joined = lines.join("\n");
        assert!(joined.contains("[a]"), "got: {}", joined);
        assert!(joined.contains("[b]"), "got: {}", joined);
        // b appears after a
        let a_pos = joined.find("[a]").unwrap();
        let b_pos = joined.find("[b]").unwrap();
        assert!(b_pos > a_pos, "b should appear below a");
    }

    #[test]
    fn test_render_loop_shows_loop_marker() {
        let graph = Graph {
            start: "review".to_string(),
            stages: vec!["review".to_string(), "revise".to_string()],
            edges: vec![
                Edge { from: "review".to_string(), to: "revise".to_string(), label: r#"verdict != "approved""#.to_string(), max_visits: Some(4), is_back_edge: false },
                Edge { from: "revise".to_string(), to: "review".to_string(), label: String::new(), max_visits: None, is_back_edge: true },
            ],
        };
        let lines = render_graph(&graph);
        let joined = lines.join("\n");
        assert!(joined.contains("loop"), "expected loop marker, got: {}", joined);
        assert!(joined.contains("[max:4]"), "expected max:4, got: {}", joined);
    }

    #[test]
    fn test_render_branch_includes_both_targets() {
        let graph = Graph {
            start: "classify".to_string(),
            stages: vec!["classify".to_string(), "approve".to_string(), "reject".to_string()],
            edges: vec![
                Edge { from: "classify".to_string(), to: "approve".to_string(), label: r#"verdict == "ok""#.to_string(), max_visits: None, is_back_edge: false },
                Edge { from: "classify".to_string(), to: "reject".to_string(), label: r#"verdict != "ok""#.to_string(), max_visits: None, is_back_edge: false },
            ],
        };
        let lines = render_graph(&graph);
        let joined = lines.join("\n");
        assert!(joined.contains("[approve]"), "got: {}", joined);
        assert!(joined.contains("[reject]"), "got: {}", joined);
    }
```

- [ ] **Step 2: Run to verify tests fail**

```bash
cargo test tui::visualizer::tests::test_render 2>&1 | tail -20
```

Expected: panics with `not yet implemented`.

- [ ] **Step 3: Implement `render_graph`**

Replace `todo!()` in `render_graph` with:

```rust
pub fn render_graph(graph: &Graph) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut rendered: std::collections::HashSet<String> = std::collections::HashSet::new();
    render_node(&graph.start, graph, &mut lines, &mut rendered, 0);
    lines
}

fn render_node(
    stage: &str,
    graph: &Graph,
    lines: &mut Vec<String>,
    rendered: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    let indent = "  ".repeat(depth);

    if rendered.contains(stage) {
        lines.push(format!("{}[{}] (already rendered above)", indent, stage));
        return;
    }
    rendered.insert(stage.to_string());

    lines.push(format!("{}[{}]", indent, stage));

    let outgoing: Vec<&Edge> = graph.edges.iter()
        .filter(|e| e.from == stage)
        .collect();

    if outgoing.is_empty() { return; }

    if outgoing.len() == 1 {
        let e = outgoing[0];
        if e.is_back_edge {
            let max_str = e.max_visits.map(|n| format!(" [max:{}]", n)).unwrap_or_default();
            lines.push(format!("{}  │", indent));
            lines.push(format!("{}  └─→ [{}] (loop ↑){}", indent, e.to, max_str));
        } else {
            if !e.label.is_empty() {
                lines.push(format!("{}  │ {}", indent, e.label));
            } else {
                lines.push(format!("{}  │", indent));
            }
            render_node(&e.to, graph, lines, rendered, depth);
        }
    } else {
        // Branch: multiple outgoing edges
        lines.push(format!("{}  │", indent));
        lines.push(format!("{}  ├── branches ──", indent));
        for (i, e) in outgoing.iter().enumerate() {
            let connector = if i + 1 == outgoing.len() { "└" } else { "├" };
            if e.is_back_edge {
                let max_str = e.max_visits.map(|n| format!(" [max:{}]", n)).unwrap_or_default();
                let label = if e.label.is_empty() { String::new() } else { format!(" ({})", e.label) };
                lines.push(format!("{}  {}─→ [{}] (loop ↑){}{}", indent, connector, e.to, label, max_str));
            } else {
                let label = if e.label.is_empty() { String::new() } else { format!(" ({})", e.label) };
                lines.push(format!("{}  {}─ {}", indent, connector, label.trim()));
                render_node(&e.to, graph, lines, rendered, depth + 2);
            }
        }
    }
}
```

- [ ] **Step 4: Run render tests**

```bash
cargo test tui::visualizer::tests::test_render 2>&1 | tail -30
```

Expected: all 4 render tests pass. Fix any failures before continuing.

- [ ] **Step 5: Run all visualizer tests**

```bash
cargo test tui::visualizer 2>&1 | tail -20
```

Expected: all 9 visualizer tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/tui/visualizer.rs
git commit -m "feat: visualizer ASCII render with branch/loop support"
```

---

## Task 4: `thruline graph` CLI command

**Files:**
- Modify: `src/cli.rs` (cmd_graph stub → real impl, already wired in Task 1)
- Test: `tests/integration.rs`

The stub in Task 1 already calls `build_graph` + `render_graph`. Now we verify it works end-to-end.

- [ ] **Step 1: Write failing integration test**

Add to `tests/integration.rs`:

```rust
#[test]
fn test_graph_renders_flowchart() {
    let dir = tempfile::tempdir().unwrap();
    let tl = write_tl(dir.path(), "flow.line", BASIC_TL);

    let out = thruline().args(["graph", tl.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // BASIC_TL has stages a and b
    assert!(stdout.contains("[a]"), "expected [a] in output: {}", stdout);
    assert!(stdout.contains("[b]"), "expected [b] in output: {}", stdout);
}

#[test]
fn test_graph_missing_file_errors() {
    let out = thruline().args(["graph", "/nonexistent/pipeline.line"]).output().unwrap();
    assert!(!out.status.success());
}
```

- [ ] **Step 2: Run to verify tests fail**

```bash
cargo test test_graph 2>&1 | tail -20
```

Expected: FAIL — `build_graph` panics with `not yet implemented`. (The cmd_graph stub is already wired but build_graph is now real — this should actually pass after Task 3. If it already passes, skip to step 4.)

- [ ] **Step 3: Verify cmd_graph stub needs no changes**

The stub from Task 1 (`cmd_graph` in `src/cli.rs`) calls `build_graph` and `render_graph`, both now real. Check it:

```rust
fn cmd_graph(file: &Path, pipeline: Option<&str>) -> anyhow::Result<()> {
    let items = load_items(file)?;
    let graph = crate::tui::visualizer::build_graph(&items, pipeline)?;
    for line in crate::tui::visualizer::render_graph(&graph) {
        println!("{}", line);
    }
    Ok(())
}
```

No changes needed if this matches what was written in Task 1.

- [ ] **Step 4: Run integration tests**

```bash
cargo test test_graph 2>&1 | tail -20
```

Expected: both tests pass.

- [ ] **Step 5: Manual smoke test**

```bash
cargo run -- graph examples/code-review/review.line
```

Expected: ASCII flowchart printed showing `[review]`, `[revise]`, `[summarize]` with branch lines.

- [ ] **Step 6: Commit**

```bash
git add tests/integration.rs
git commit -m "feat: thruline graph command — ASCII flowchart to stdout"
```

---

## Task 5: App state — file browser

**Files:**
- Modify: `src/tui/app.rs`

The file browser starts at cwd, lists `..`, subdirectories, then `*.line` files. Navigating into a directory or `..` updates the listing.

- [ ] **Step 1: Write failing tests**

Replace `src/tui/app.rs` with:

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Parent,             // ".." navigate up
    Dir(PathBuf),       // subdirectory
    LineFile(PathBuf),  // *.line file
}

impl Entry {
    pub fn display_name(&self) -> String {
        match self {
            Entry::Parent => "..".to_string(),
            Entry::Dir(p) => format!("{}/", p.file_name().unwrap_or_default().to_string_lossy()),
            Entry::LineFile(p) => p.file_name().unwrap_or_default().to_string_lossy().to_string(),
        }
    }
}

#[derive(Debug)]
pub struct FileBrowser {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    pub selected_file: Option<PathBuf>,
}

impl FileBrowser {
    pub fn new(cwd: PathBuf) -> Self {
        let mut fb = FileBrowser { cwd: cwd.clone(), entries: vec![], cursor: 0, selected_file: None };
        fb.reload();
        fb
    }

    pub fn reload(&mut self) {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        if let Ok(read) = std::fs::read_dir(&self.cwd) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().map_or(false, |ext| ext == "line") {
                    files.push(path);
                }
            }
        }
        dirs.sort();
        files.sort();
        self.entries = vec![Entry::Parent];
        self.entries.extend(dirs.into_iter().map(Entry::Dir));
        self.entries.extend(files.into_iter().map(Entry::LineFile));
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
        // Clear selected file if it's no longer in this dir
        if let Some(sel) = &self.selected_file {
            if sel.parent() != Some(&self.cwd) {
                self.selected_file = None;
            }
        }
    }

    pub fn navigate_up(&mut self) { self.cursor = if self.cursor == 0 { self.entries.len() - 1 } else { self.cursor - 1 }; }
    pub fn navigate_down(&mut self) { self.cursor = if self.cursor + 1 >= self.entries.len() { 0 } else { self.cursor + 1 }; }

    /// Enter the currently selected entry. Returns true if a .line file was selected.
    pub fn enter(&mut self) -> bool {
        match self.entries.get(self.cursor).cloned() {
            Some(Entry::Parent) => {
                if let Some(parent) = self.cwd.parent().map(|p| p.to_path_buf()) {
                    self.cwd = parent;
                    self.cursor = 0;
                    self.selected_file = None;
                    self.reload();
                }
                false
            }
            Some(Entry::Dir(path)) => {
                self.cwd = path;
                self.cursor = 0;
                self.selected_file = None;
                self.reload();
                false
            }
            Some(Entry::LineFile(path)) => {
                self.selected_file = Some(path);
                true
            }
            None => false,
        }
    }
}

// ── TUI mode ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TuiMode {
    FileBrowser,  // a .line file is selected → show flowchart
    RunList,      // no file selected → show run list
}

// ── Modal ────────────────────────────────────────────────────────────────

pub const DRIVERS: &[&str] = &["stdio", "anthropic", "mock", "ollama", "openai", "bedrock", "vertex"];

#[derive(Debug)]
pub struct ModalState {
    pub file: PathBuf,
    pub driver_idx: usize,
    pub input_keys: Vec<String>,   // names of declared pipeline inputs
    pub input_values: Vec<String>, // parallel to input_keys
    pub pipeline_names: Vec<String>,
    pub pipeline_idx: usize,
    pub focused_field: usize,      // 0=driver, 1..=input_keys.len()=inputs, last=pipeline selector
}

impl ModalState {
    pub fn total_fields(&self) -> usize {
        1 + self.input_keys.len() + if self.pipeline_names.len() > 1 { 1 } else { 0 }
    }

    pub fn next_field(&mut self) { self.focused_field = (self.focused_field + 1) % self.total_fields(); }
    pub fn prev_field(&mut self) { self.focused_field = if self.focused_field == 0 { self.total_fields() - 1 } else { self.focused_field - 1 }; }

    pub fn cycle_driver_forward(&mut self) { self.driver_idx = (self.driver_idx + 1) % DRIVERS.len(); }
    pub fn cycle_driver_backward(&mut self) { self.driver_idx = if self.driver_idx == 0 { DRIVERS.len() - 1 } else { self.driver_idx - 1 }; }

    pub fn driver(&self) -> &str { DRIVERS[self.driver_idx] }
    pub fn selected_pipeline(&self) -> Option<&str> {
        if self.pipeline_names.len() > 1 { Some(&self.pipeline_names[self.pipeline_idx]) } else { None }
    }

    pub fn build_launch_args(&self) -> Vec<String> {
        let mut args = vec![
            "run".to_string(),
            self.file.to_string_lossy().to_string(),
            "--driver".to_string(),
            self.driver().to_string(),
        ];
        for (k, v) in self.input_keys.iter().zip(self.input_values.iter()) {
            if !v.is_empty() {
                args.push("--input".to_string());
                args.push(format!("{}={}", k, v));
            }
        }
        if let Some(name) = self.selected_pipeline() {
            args.push("--pipeline".to_string());
            args.push(name.to_string());
        }
        args
    }
}

// ── App ──────────────────────────────────────────────────────────────────

pub struct App {
    pub browser: FileBrowser,
    pub run_list: Vec<crate::runtime::state::RunState>,
    pub selected_run: usize,
    pub event_logs: std::collections::HashMap<String, Vec<String>>,
    pub modal: Option<ModalState>,
    pub flowchart_lines: Vec<String>,
    pub flowchart_cursor: usize,
    pub graph_stages: Vec<String>,  // ordered stage names for cursor navigation
    pub event_rx: tokio::sync::mpsc::Receiver<(String, String)>,  // (run_id, ndjson_line)
    pub delete_confirm: Option<String>,  // run_id pending delete confirmation
    pub pane_focus: PaneFocus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaneFocus {
    Files,
    Middle,
    Detail,
}

impl App {
    pub fn new(event_rx: tokio::sync::mpsc::Receiver<(String, String)>) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;
        let run_list = crate::runtime::state::list_runs().unwrap_or_default();
        Ok(App {
            browser: FileBrowser::new(cwd),
            run_list,
            selected_run: 0,
            event_logs: std::collections::HashMap::new(),
            modal: None,
            flowchart_lines: vec![],
            flowchart_cursor: 0,
            graph_stages: vec![],
            event_rx,
            delete_confirm: None,
            pane_focus: PaneFocus::Files,
        })
    }

    pub fn mode(&self) -> TuiMode {
        if self.browser.selected_file.is_some() { TuiMode::FileBrowser } else { TuiMode::RunList }
    }

    pub async fn tick(&mut self) {
        // Drain event channel
        while let Ok((run_id, line)) = self.event_rx.try_recv() {
            self.event_logs.entry(run_id).or_default().push(line);
        }
        // Refresh run list
        if let Ok(runs) = crate::runtime::state::list_runs() {
            self.run_list = runs;
        }
    }

    pub fn update_flowchart(&mut self) {
        let Some(file) = &self.browser.selected_file else { return; };
        let file = file.clone();
        match crate::cli::load_items(&file) {
            Ok(items) => {
                let pipeline_name = self.modal.as_ref().and_then(|m| m.selected_pipeline());
                match crate::tui::visualizer::build_graph(&items, pipeline_name) {
                    Ok(graph) => {
                        self.graph_stages = graph.stages.clone();
                        self.flowchart_lines = crate::tui::visualizer::render_graph(&graph);
                        self.flowchart_cursor = 0;
                    }
                    Err(_) => {
                        self.flowchart_lines = vec!["(could not build graph)".to_string()];
                        self.graph_stages = vec![];
                    }
                }
            }
            Err(_) => {
                self.flowchart_lines = vec!["(could not parse file)".to_string()];
                self.graph_stages = vec![];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_browser_lists_line_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.line"), "").unwrap();
        std::fs::write(dir.path().join("b.line"), "").unwrap();
        std::fs::write(dir.path().join("readme.md"), "").unwrap();
        let fb = FileBrowser::new(dir.path().to_path_buf());
        let names: Vec<_> = fb.entries.iter().map(|e| e.display_name()).collect();
        assert!(names.contains(&"..".to_string()));
        assert!(names.contains(&"a.line".to_string()));
        assert!(names.contains(&"b.line".to_string()));
        assert!(!names.contains(&"readme.md".to_string()));
    }

    #[test]
    fn test_file_browser_navigate_down_wraps() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.line"), "").unwrap();
        let mut fb = FileBrowser::new(dir.path().to_path_buf());
        assert_eq!(fb.entries.len(), 2); // ".." and "a.line"
        fb.navigate_down(); // cursor=1
        fb.navigate_down(); // wraps to 0
        assert_eq!(fb.cursor, 0);
    }

    #[test]
    fn test_file_browser_enter_selects_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.line"), "").unwrap();
        let mut fb = FileBrowser::new(dir.path().to_path_buf());
        fb.navigate_down(); // cursor on a.line (index 1, past "..")
        let selected = fb.enter();
        assert!(selected);
        assert!(fb.selected_file.is_some());
    }

    #[test]
    fn test_file_browser_navigate_up_changes_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("sub");
        std::fs::create_dir(&subdir).unwrap();
        let mut fb = FileBrowser::new(subdir.clone());
        // cursor=0 is "..", enter navigates up
        fb.enter();
        assert_eq!(fb.cwd, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn test_modal_build_launch_args_basic() {
        let modal = ModalState {
            file: PathBuf::from("/tmp/test.line"),
            driver_idx: 1,  // anthropic
            input_keys: vec!["code".to_string()],
            input_values: vec!["file:///tmp/code.rs".to_string()],
            pipeline_names: vec!["p".to_string()],
            pipeline_idx: 0,
            focused_field: 0,
        };
        let args = modal.build_launch_args();
        assert_eq!(args[0], "run");
        assert!(args.contains(&"--driver".to_string()));
        assert!(args.contains(&"anthropic".to_string()));
        assert!(args.contains(&"--input".to_string()));
        assert!(args.contains(&"code=file:///tmp/code.rs".to_string()));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test tui::app 2>&1 | tail -20
```

Expected: tests fail because the test helper creates `FileBrowser` and `ModalState` directly, but the implementations are not tested yet. The code should compile cleanly.

- [ ] **Step 3: Run tests**

```bash
cargo test tui::app 2>&1 | tail -30
```

Expected: all 5 app tests pass. Fix any failures before continuing.

- [ ] **Step 4: Commit**

```bash
git add src/tui/app.rs
git commit -m "feat: TUI app state — file browser, modal, run list scaffold"
```

---

## Task 6: Runner — subprocess spawn and NDJSON streaming

**Files:**
- Modify: `src/tui/runner.rs`

The runner spawns `thruline run ...` as a child process and streams its stdout lines to a channel.

- [ ] **Step 1: Write failing tests**

Replace `src/tui/runner.rs` with:

```rust
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::Sender;

/// Spawn a `thruline run` child process and stream each NDJSON line to `tx` as `(run_id, line)`.
/// The run_id is extracted from the first `pipeline_start` event.
pub async fn spawn_run(
    file: &Path,
    driver: &str,
    inputs: &[String],
    pipeline: Option<&str>,
    tx: Sender<(String, String)>,
) -> anyhow::Result<()> {
    let bin = std::env::current_exe()?;
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.arg("run").arg(file).arg("--driver").arg(driver);
    for input in inputs {
        cmd.arg("--input").arg(input);
    }
    if let Some(name) = pipeline {
        cmd.arg("--pipeline").arg(name);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    let mut run_id = String::from("unknown");

    while let Ok(Some(line)) = lines.next_line().await {
        // Try to extract run_id from pipeline_start
        if run_id == "unknown" {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                if v["event"] == "pipeline_start" {
                    if let Some(id) = v["run_id"].as_str() {
                        run_id = id.to_string();
                    }
                }
            }
        }
        let _ = tx.send((run_id.clone(), line)).await;
    }

    child.wait().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_spawn_run_streams_pipeline_start() {
        // Use the integration test binary path trick
        let bin = std::env::current_exe().unwrap();
        // Find the thruline binary adjacent to test binary
        let bin_dir = bin.parent().unwrap();
        let thruline_bin = bin_dir.join("thruline");
        if !thruline_bin.exists() {
            // Skip if not built
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let content = r#"
runner r { model: claude-sonnet-4-6  system: "test" }
stage a { out: v as value  runner: r }
thruline p { start: a  routes {} }
"#;
        let tl_path = dir.path().join("test.line");
        std::fs::write(&tl_path, content).unwrap();

        let (tx, mut rx) = mpsc::channel(64);
        let _ = spawn_run(&tl_path, "stdio", &[], None, tx).await;

        let mut events = vec![];
        while let Ok((_id, line)) = rx.try_recv() {
            events.push(line);
        }
        assert!(!events.is_empty(), "no events received");
        assert!(events[0].contains("pipeline_start"), "first event: {}", events[0]);
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test tui::runner 2>&1 | tail -20
```

Expected: test passes (or skips if binary not built — both are OK). If compile error, fix before continuing.

- [ ] **Step 3: Commit**

```bash
git add src/tui/runner.rs
git commit -m "feat: TUI runner — subprocess spawn + NDJSON streaming"
```

---

## Task 7: UI rendering — three panes

**Files:**
- Modify: `src/tui/ui.rs`

Ratatui render function: split terminal into three columns, render each pane based on App state. No automated tests for rendering — manual verification via `thruline -i`.

- [ ] **Step 1: Implement render**

Replace `src/tui/ui.rs` with:

```rust
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use crate::runtime::state::{RunState, RunStatus};
use super::app::{App, TuiMode, PaneFocus};

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(40),
            Constraint::Percentage(35),
        ])
        .split(f.area());

    render_files(f, app, chunks[0]);
    match app.mode() {
        TuiMode::FileBrowser => render_flowchart(f, app, chunks[1]),
        TuiMode::RunList     => render_run_list(f, app, chunks[1]),
    }
    match app.mode() {
        TuiMode::FileBrowser => render_stage_detail(f, app, chunks[2]),
        TuiMode::RunList     => render_event_log(f, app, chunks[2]),
    }

    if app.modal.is_some() {
        render_modal(f, app, f.area());
    }

    if let Some(run_id) = &app.delete_confirm {
        render_confirm_bar(f, run_id, f.area());
    }
}

fn pane_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn render_files(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane_focus == PaneFocus::Files;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Files ")
        .border_style(pane_style(focused));

    let items: Vec<ListItem> = app.browser.entries.iter().enumerate().map(|(i, e)| {
        let name = e.display_name();
        let style = if Some(i) == app.browser.selected_file.as_ref().and_then(|sel| {
            app.browser.entries.iter().position(|entry| {
                if let super::app::Entry::LineFile(p) = entry { p == sel } else { false }
            })
        }) {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else if i == app.browser.cursor && focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        ListItem::new(name).style(style)
    }).collect();

    let mut state = ListState::default();
    if focused { state.select(Some(app.browser.cursor)); }

    f.render_stateful_widget(List::new(items).block(block), area, &mut state);
}

fn render_flowchart(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane_focus == PaneFocus::Middle;
    let title = app.browser.selected_file
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| format!(" Flow: {} ", n.to_string_lossy()))
        .unwrap_or_else(|| " Flow ".to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(pane_style(focused));

    let lines: Vec<Line> = app.flowchart_lines.iter().enumerate().map(|(i, l)| {
        let style = if i == app.flowchart_cursor && focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        Line::styled(l.clone(), style)
    }).collect();

    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

fn status_badge(status: &RunStatus) -> (&'static str, Color) {
    match status {
        RunStatus::Done => ("● done", Color::Green),
        RunStatus::Failed(_) => ("○ failed", Color::Red),
        RunStatus::Running => ("◉ running", Color::Yellow),
        RunStatus::AwaitingResume { .. } => ("◉ awaiting", Color::Yellow),
        RunStatus::ParallelAwait { .. } => ("◉ parallel", Color::Yellow),
    }
}

fn render_run_list(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane_focus == PaneFocus::Middle;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Runs ")
        .border_style(pane_style(focused));

    let items: Vec<ListItem> = app.run_list.iter().enumerate().map(|(i, run)| {
        let (badge, color) = status_badge(&run.status);
        let line = format!("{:<10} {:<20} {}", badge, run.pipeline, run.started.format("%m-%d %H:%M"));
        let style = if i == app.selected_run && focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(color)
        };
        ListItem::new(line).style(style)
    }).collect();

    let mut state = ListState::default();
    if focused { state.select(Some(app.selected_run)); }

    f.render_stateful_widget(List::new(items).block(block), area, &mut state);
}

fn render_stage_detail(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane_focus == PaneFocus::Detail;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Stage Detail ")
        .border_style(pane_style(focused));

    let content = if app.graph_stages.is_empty() {
        "Select a .line file to inspect".to_string()
    } else {
        app.graph_stages.get(app.flowchart_cursor)
            .map(|stage_name| format_stage_detail(stage_name, app))
            .unwrap_or_default()
    };

    f.render_widget(Paragraph::new(content).block(block).wrap(Wrap { trim: false }), area);
}

fn format_stage_detail(stage_name: &str, app: &App) -> String {
    let Some(file) = &app.browser.selected_file else { return String::new() };
    let Ok(items) = crate::cli::load_items(file) else { return String::new() };

    let mut out = format!("stage: {}\n", stage_name);
    for item in &items {
        if let crate::ast::TlItem::Stage(s) = item {
            if s.name != stage_name { continue; }
            if let Some(r) = &s.runner { out.push_str(&format!("runner: {}\n", r)); }
            if !s.inputs.is_empty() {
                out.push_str("in:\n");
                for a in &s.inputs {
                    let opt = if a.optional { "?" } else { "" };
                    let kind = match a.kind { crate::ast::ArtifactKind::Path => "path", crate::ast::ArtifactKind::Value => "value" };
                    out.push_str(&format!("  {}{} as {}\n", a.name, opt, kind));
                }
            }
            if !s.outputs.is_empty() {
                out.push_str("out:\n");
                for a in &s.outputs {
                    let kind = match a.kind { crate::ast::ArtifactKind::Path => "path", crate::ast::ArtifactKind::Value => "value" };
                    let constraint = a.value_constraint.as_ref()
                        .map(|c| format!(" in [{}]", c.iter().map(|v| format!("\"{}\"", v)).collect::<Vec<_>>().join(", ")))
                        .unwrap_or_default();
                    out.push_str(&format!("  {} as {}{}\n", a.name, kind, constraint));
                }
            }
            if let Some(p) = &s.prompt {
                let src = match p { crate::ast::PromptSource::Inline(s) => s.chars().take(40).collect::<String>(), crate::ast::PromptSource::File(f) => format!("file({})", f) };
                out.push_str(&format!("prompt: {}\n", src));
            }
        }
    }
    out
}

fn render_event_log(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.pane_focus == PaneFocus::Detail;
    let run = app.run_list.get(app.selected_run);
    let title = run.map(|r| format!(" Events: {} ", r.run_id)).unwrap_or_else(|| " Events ".to_string());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(pane_style(focused));

    let content = if let Some(run) = run {
        app.event_logs.get(&run.run_id)
            .map(|lines| lines.iter().map(|l| format_event_line(l)).collect::<Vec<_>>().join("\n"))
            .unwrap_or_else(|| "(no events yet)".to_string())
    } else {
        "(no run selected)".to_string()
    };

    f.render_widget(Paragraph::new(content).block(block).wrap(Wrap { trim: false }), area);
}

fn format_event_line(raw: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        let event = v["event"].as_str().unwrap_or("?");
        let stage = v["stage"].as_str().map(|s| format!(" {}", s)).unwrap_or_default();
        return format!("✓ {}{}", event, stage);
    }
    raw.to_string()
}

fn render_confirm_bar(f: &mut Frame, run_id: &str, area: Rect) {
    let msg = format!(" Delete {}? [y/n] ", run_id);
    let bar_area = Rect { x: area.x, y: area.height.saturating_sub(1), width: area.width, height: 1 };
    f.render_widget(
        Paragraph::new(msg).style(Style::default().bg(Color::Red).fg(Color::White)),
        bar_area,
    );
}

pub fn render_modal(f: &mut Frame, app: &App, area: Rect) {
    let Some(modal) = &app.modal else { return };

    let width = 50u16.min(area.width.saturating_sub(4));
    let height = (4 + modal.input_keys.len() as u16 + 3).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect { x, y, width, height };

    // Clear background
    f.render_widget(ratatui::widgets::Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Launch: {} ", modal.file.file_name().unwrap_or_default().to_string_lossy()))
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let mut lines: Vec<Line> = Vec::new();

    let driver_style = if modal.focused_field == 0 { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
    lines.push(Line::from(vec![
        Span::raw("driver: "),
        Span::styled(format!("[ {} ]", modal.driver()), driver_style),
    ]));

    for (i, key) in modal.input_keys.iter().enumerate() {
        let val = &modal.input_values[i];
        let style = if modal.focused_field == i + 1 { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::raw(format!("{}: ", key)),
            Span::styled(format!("{}_", val), style),
        ]));
    }

    if modal.pipeline_names.len() > 1 {
        let field_idx = modal.input_keys.len() + 1;
        let style = if modal.focused_field == field_idx { Style::default().add_modifier(Modifier::REVERSED) } else { Style::default() };
        lines.push(Line::from(vec![
            Span::raw("pipeline: "),
            Span::styled(modal.pipeline_names[modal.pipeline_idx].clone(), style),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled("tab:next  enter:launch  esc:cancel", Style::default().fg(Color::DarkGray)));

    f.render_widget(Paragraph::new(lines), inner);
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Expected: no errors. Fix any type mismatches.

- [ ] **Step 3: Commit**

```bash
git add src/tui/ui.rs
git commit -m "feat: TUI ratatui render — three panes, modal overlay"
```

---

## Task 8: Event loop, key handling, and `cmd_tui`

**Files:**
- Modify: `src/tui/mod.rs`

Wire the event loop: crossterm raw mode, ratatui terminal, async key events + tick, all keybindings.

- [ ] **Step 1: Implement cmd_tui**

Replace `src/tui/mod.rs` with:

```rust
pub mod app;
pub mod runner;
pub mod ui;
pub mod visualizer;

use std::io::stdout;
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use app::{App, ModalState, PaneFocus, DRIVERS};
use crate::ast::TlItem;

pub async fn cmd_tui() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let (tx, rx) = mpsc::channel::<(String, String)>(256);
    let mut app = App::new(rx)?;

    let mut reader = crossterm::event::EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut should_quit = false;

    while !should_quit {
        terminal.draw(|f| ui::render(f, &app))?;

        tokio::select! {
            _ = tick.tick() => {
                app.tick().await;
            }
            maybe_ev = reader.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_ev {
                    should_quit = handle_key(&mut app, key, &tx).await;
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn handle_key(app: &mut App, key: KeyEvent, tx: &mpsc::Sender<(String, String)>) -> bool {
    // Quit always works
    if key.code == KeyCode::Char('q') && app.modal.is_none() {
        return true;
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }

    // Delete confirmation
    if let Some(run_id) = app.delete_confirm.clone() {
        match key.code {
            KeyCode::Char('y') => {
                let run_dir = crate::runtime::state::runs_dir().join(&run_id);
                let _ = std::fs::remove_dir_all(run_dir);
                app.delete_confirm = None;
                app.tick().await;
            }
            _ => { app.delete_confirm = None; }
        }
        return false;
    }

    // Modal handling
    if let Some(modal) = &mut app.modal {
        match key.code {
            KeyCode::Esc => { app.modal = None; }
            KeyCode::Tab => { app.modal.as_mut().unwrap().next_field(); }
            KeyCode::BackTab => { app.modal.as_mut().unwrap().prev_field(); }
            KeyCode::Up => {
                let m = app.modal.as_mut().unwrap();
                if m.focused_field == 0 { m.cycle_driver_backward(); }
            }
            KeyCode::Down => {
                let m = app.modal.as_mut().unwrap();
                if m.focused_field == 0 { m.cycle_driver_forward(); }
            }
            KeyCode::Char(c) => {
                let m = app.modal.as_mut().unwrap();
                let field = m.focused_field;
                if field > 0 && field <= m.input_keys.len() {
                    m.input_values[field - 1].push(c);
                }
            }
            KeyCode::Backspace => {
                let m = app.modal.as_mut().unwrap();
                let field = m.focused_field;
                if field > 0 && field <= m.input_keys.len() {
                    m.input_values[field - 1].pop();
                }
            }
            KeyCode::Enter => {
                let m = app.modal.as_ref().unwrap();
                let args = m.build_launch_args();
                // args[0] is "run", rest are the actual args
                let file = m.file.clone();
                let driver = m.driver().to_string();
                let inputs: Vec<String> = m.input_keys.iter().zip(m.input_values.iter())
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect();
                let pipeline = m.selected_pipeline().map(|s| s.to_string());
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    let _ = runner::spawn_run(&file, &driver, &inputs, pipeline.as_deref(), tx_clone).await;
                });
                app.modal = None;
            }
            _ => {}
        }
        return false;
    }

    // Normal navigation
    match key.code {
        KeyCode::Char('1') => { app.pane_focus = PaneFocus::Files; }
        KeyCode::Char('2') => { app.pane_focus = PaneFocus::Middle; }
        KeyCode::Char('3') => { app.pane_focus = PaneFocus::Detail; }
        KeyCode::Tab => {
            app.pane_focus = match app.pane_focus {
                PaneFocus::Files  => PaneFocus::Middle,
                PaneFocus::Middle => PaneFocus::Detail,
                PaneFocus::Detail => PaneFocus::Files,
            };
        }
        KeyCode::Up => match app.pane_focus {
            PaneFocus::Files => { app.browser.navigate_up(); }
            PaneFocus::Middle => {
                if app.mode() == app::TuiMode::RunList {
                    if app.selected_run > 0 { app.selected_run -= 1; }
                } else {
                    if app.flowchart_cursor > 0 { app.flowchart_cursor -= 1; }
                }
            }
            PaneFocus::Detail => {}
        },
        KeyCode::Down => match app.pane_focus {
            PaneFocus::Files => { app.browser.navigate_down(); }
            PaneFocus::Middle => {
                if app.mode() == app::TuiMode::RunList {
                    if app.selected_run + 1 < app.run_list.len() { app.selected_run += 1; }
                } else {
                    let max = app.flowchart_lines.len().saturating_sub(1);
                    if app.flowchart_cursor < max { app.flowchart_cursor += 1; }
                }
            }
            PaneFocus::Detail => {}
        },
        KeyCode::Enter => {
            if app.pane_focus == PaneFocus::Files {
                let was_selected = app.browser.enter();
                if was_selected {
                    app.update_flowchart();
                    app.pane_focus = PaneFocus::Middle;
                }
            }
        }
        KeyCode::Char('r') => {
            if app.pane_focus == PaneFocus::Files {
                if let Some(file) = app.browser.selected_file.clone() {
                    if let Ok(items) = crate::cli::load_items(&file) {
                        let pipeline_names: Vec<String> = items.iter()
                            .filter_map(|i| if let TlItem::Pipeline(p) = i { Some(p.name.clone()) } else { None })
                            .collect();
                        let input_keys: Vec<String> = items.iter()
                            .filter_map(|i| if let TlItem::Pipeline(p) = i { Some(p) } else { None })
                            .next()
                            .map(|p| p.inputs.iter().map(|inp| inp.name.clone()).collect())
                            .unwrap_or_default();
                        let n = input_keys.len();
                        app.modal = Some(ModalState {
                            file,
                            driver_idx: 0,
                            input_keys,
                            input_values: vec![String::new(); n],
                            pipeline_names,
                            pipeline_idx: 0,
                            focused_field: 0,
                        });
                    }
                }
            }
        }
        KeyCode::Char('d') => {
            if app.pane_focus == PaneFocus::Middle {
                if let Some(run) = app.run_list.get(app.selected_run) {
                    app.delete_confirm = Some(run.run_id.clone());
                }
            }
        }
        _ => {}
    }

    false
}
```

- [ ] **Step 2: Verify compile**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Expected: no errors. Fix any import or type mismatches.

- [ ] **Step 3: Smoke test — launch and quit**

```bash
cargo run -- -i
```

Expected: TUI launches (three columns visible). Press `q` to quit cleanly. The terminal should return to normal after quit.

- [ ] **Step 4: Smoke test — file browser**

```bash
cargo run -- -i
```

- Press `↓` to move cursor down in Files pane
- Press `Enter` on a `.line` file — middle pane should show flowchart
- Press `r` — modal should appear
- Press `Esc` — modal closes

- [ ] **Step 5: Commit**

```bash
git add src/tui/mod.rs
git commit -m "feat: TUI event loop, key handling, cmd_tui entry point"
```

---

## Task 9: Run all tests, fix regressions, final cleanup

**Files:**
- Modify: any file with failures
- Modify: `src/tui/app.rs` — fix PaneFocus import used in mod.rs

- [ ] **Step 1: Run full test suite**

```bash
cargo test 2>&1 | tail -40
```

Expected: all tests pass (including existing integration tests). Fix any regressions.

- [ ] **Step 2: Run the graph command on all examples**

```bash
cargo run -- graph examples/sentiment/pipeline.line
cargo run -- graph examples/code-review/review.line
cargo run -- graph examples/multi_reviewer/review.line
```

Expected: each prints an ASCII flowchart to stdout. Verify stages match the `.line` file content.

- [ ] **Step 3: Verify -i flag in help**

```bash
cargo run -- --help
```

Expected: shows `-i, --interactive` in the flags. Shows `graph` as a subcommand.

- [ ] **Step 4: Update README with new commands**

In `README.md`, find the CLI reference section and add `graph` and `-i`/`--interactive`. The CLI table currently shows `validate`, `inspect`, `run`, `runs`, `status`, `resume`, `serve`, `lsp`. Add:

```markdown
| `thruline graph <file.line>`                    | Render pipeline as ASCII flowchart         |
| `thruline -i` / `thruline --interactive`        | Launch interactive TUI                     |
```

- [ ] **Step 5: Update LANGUAGE.md CLI section**

In `docs/LANGUAGE.md`, the CLI section lists commands. Add after the existing entries:

```
thruline graph    <file.line>                # Render pipeline as ASCII flowchart
thruline -i                                  # Launch interactive TUI
```

- [ ] **Step 6: Commit**

```bash
git add README.md docs/LANGUAGE.md
git commit -m "docs: document graph command and -i flag"
```

- [ ] **Step 7: Run tests one final time**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 8: Final commit**

```bash
git add -A
git status  # verify only expected files
git commit -m "feat: thruline TUI and graph visualizer complete"
```

---

## Implementation Notes

**`TlItem::Pipeline` not `TlItem::Thruline`:** The AST type is `TlItem::Pipeline(PipelineDecl)` even though the `.line` keyword is `thruline`. This is correct — don't rename it.

**`cmd_graph` is sync, not async:** The `Commands::Graph` match arm calls `cmd_graph(&file, pipeline.as_deref())` which returns `anyhow::Result<()>` — it's not async. Keep it that way.

**ratatui 0.29 API:** Uses `f.area()` (not `f.size()`). `Constraint::Percentage`, `Layout::default()`, `Block::default()`. Import from `ratatui::{layout, style, text, widgets, Frame}`.

**crossterm event-stream feature:** Required for `EventStream::new()` — make sure `crossterm = { version = "0.28", features = ["event-stream"] }` in Cargo.toml.

**`build_graph` back-edge detection:** Uses DFS with a visited set and an in-stack set. Edges where the target is already in the DFS stack are marked `is_back_edge: true`.
