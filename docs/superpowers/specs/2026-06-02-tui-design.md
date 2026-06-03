# Thruline TUI + Graph Visualizer — Design Spec

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an interactive terminal UI (`thruline -i`) for browsing `.line` files, visualizing pipeline graphs, launching runs, and monitoring live run events; plus a standalone CLI command (`thruline graph <file>`) that renders the ASCII flowchart to stdout.

**Architecture:** ratatui + crossterm for the TUI layer. Runs are launched by spawning `thruline run ...` as child processes and streaming their NDJSON stdout back via a tokio channel. A shared `visualizer` module builds a directed graph from the AST and renders it as ASCII art — used both by the TUI pane and the `graph` CLI command. The existing `thruline` lib is unchanged; the TUI only uses `list_runs()`, `RunState`, and `load_items()`.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, tokio (already present), existing thruline lib.

---

## Invocation

```
thruline -i
thruline --interactive
thruline graph <file.line>
```

`-i` / `--interactive` is a flag on the root `Cli` struct. When present, launches the TUI instead of requiring a subcommand. `graph` is a new subcommand that renders the flowchart to stdout and exits.

---

## TUI Layout

Three-column layout, always visible:

```
┌── Files ──────┬── Flow / Runs ──────────┬── Detail ───────────────┐
│               │                          │                          │
│  directory    │  flowchart (file mode)   │  stage details           │
│  tree with    │  ── or ──                │  ── or ──                │
│  .line files  │  run list (run mode)     │  event log               │
│               │                          │                          │
└───────────────┴──────────────────────────┴──────────────────────────┘
```

**Left pane — file browser**
- Starts at `cwd`. Entries: `..` (navigate up), subdirectories, `*.line` files.
- `↑↓` to navigate, `enter` to enter directory or select file.
- Selecting a `.line` file switches the middle + right panes to *file mode*.

**Middle pane — context-dependent**
- *File mode*: ASCII flowchart of the selected file's thruline (see Visualizer).
- *Run mode*: list of all runs from `list_runs()`, sorted newest-first, with status badges.
- Mode is determined by left-pane selection: selecting a `.line` file switches to file mode; selecting a directory or `..` (deselecting any file) switches back to run mode. On initial launch, no file is selected, so middle pane starts in run mode.

**Right pane — context-dependent**
- *File mode*: stage/route details for the flowchart node currently selected in the middle pane.
- *Run mode*: NDJSON event log for the run selected in the middle pane.

**Pane focus**

| Key | Action |
|---|---|
| `tab` | Cycle focus: Files → Middle → Detail → Files |
| `1` | Focus Files pane |
| `2` | Focus Middle pane |
| `3` | Focus Detail pane |

---

## Visualizer

Shared module used by both the TUI middle pane and `thruline graph`.

**Graph build:** parse the `.line` file via `load_items()`, extract the `thruline` declaration, walk its `routes` to build a list of `(source_stage, condition, target_stage, max_visits)` edges.

**ASCII render:** stages rendered as `[name]` nodes, routes as vertical/diagonal lines with condition labels. Loops (a stage that routes back to itself or to an ancestor) are detected and rendered as `(loop ↑)` rather than re-drawing the cycle. `[max:N]` annotations shown on loop edges.

Example output for `review.line`:

```
      [review]
          │
     ┌────┴────┐
 approved    rejected
     │            │
[summarize]   [revise] ──[max:4]
                   │
              [review]
               (loop ↑)
```

**TUI navigation:** `↑↓` in the middle pane moves a cursor through stages in topological order (depth-first, loops treated as leaf). Selected stage is highlighted with reverse video. Right pane updates live.

**`thruline graph` output:** same renderer, written to stdout. Supports `--pipeline <name>` when the file declares multiple thrulines.

---

## Stage Detail Pane (file mode)

When a stage node is selected in the flowchart:

```
stage: review
runner: reviewer
  model: claude-sonnet-4-6
  system: file("prompts/reviewer.md")
  temperature: 0.2

in:
  code     as path
  notes?   as value        (optional)

out:
  verdict  as value  in ["approved","rejected"]
  feedback as value

prompt: file("prompts/review-task.md")

routes from here:
  verdict == "approved"  →  summarize
  verdict != "approved"  →  revise  [max:4]
```

When a route edge is selected (future: clicking on route labels), the detail pane shows route metadata. For now, selecting a stage node shows outgoing routes at the bottom of the detail pane.

---

## Launch Modal

Triggered by pressing `r` with a `.line` file focused in the left pane.

- Overlays the three-column layout with a centered modal box.
- Fields (in tab order):
  1. `driver` — cycles through: `stdio`, `anthropic`, `mock`, `ollama`, `openai`, `bedrock`, `vertex`
  2. One text input per declared pipeline `input` (parsed from the file's AST)
  3. `--pipeline` selector (shown only when file declares multiple thrulines)
- `tab` / `shift-tab` — move between fields
- `enter` — launch: calls `thruline run <file> --driver <x> [--input k=v ...]` as subprocess
- `esc` — cancel

On launch: modal closes, middle pane switches to run mode, new run appears at top of run list.

---

## Run Execution & Event Streaming

When a run is launched from the TUI:

1. `runner.rs` spawns `Command::new("thruline").args(["run", file, "--driver", driver, ...]).stdout(Stdio::piped())`
2. A tokio task reads lines from the child's stdout, parses each as `ThrulineEvent` (using existing `serde_json`), and sends to an `mpsc::channel`.
3. `App::tick()` (every 50ms) drains the channel, appending events to the run's event log in `App` state.
4. `list_runs()` is called on each tick to refresh the run list — picks up status changes written to `~/.thruline/runs/`.

---

## Run List (run mode)

Middle pane when in run mode:

```
● run-abc  code_review   done        2026-06-02 14:32
◉ run-def  sentiment     awaiting    2026-06-02 14:30
○ run-ghi  pr_triage     failed      2026-06-02 14:25
```

Status badges: `●` done (green), `◉` awaiting/running (yellow), `○` failed (red).

`↑↓` to select, right pane shows event log.

**Delete run:** `d` on selected run → confirmation line appears at bottom of middle pane: `delete run-abc? [y/n]`. `y` deletes `~/.thruline/runs/<id>/` and removes from list.

---

## Resume (out of scope)

When a run is in `AwaitingResume` state, the TUI shows it as awaiting but does not support providing artifact values inline. Resume is handled via the existing `thruline resume` CLI command.

---

## Keybindings (full reference)

| Key | Context | Action |
|---|---|---|
| `tab` | any | Cycle pane focus |
| `1` / `2` / `3` | any | Jump to Files / Middle / Detail |
| `↑` `↓` | Files, Middle | Navigate |
| `enter` | Files | Enter directory / select file |
| `r` | Files (.line selected) | Open launch modal |
| `tab` | Modal | Next field |
| `shift-tab` | Modal | Previous field |
| `↑` `↓` | Modal driver field | Cycle driver |
| `enter` | Modal | Launch run |
| `esc` | Modal | Cancel |
| `d` | Middle (run selected) | Delete run (confirm with `y`) |
| `q` | any | Quit |

---

## New Files

```
src/tui/
  mod.rs        — entry point: cmd_tui(), main event loop, channel setup
  app.rs        — App struct: file browser state, run list, event logs, flowchart cursor, modal state
  ui.rs         — ratatui render functions: three panes, launch modal overlay
  visualizer.rs — graph build from AST + ASCII render (shared with cmd_graph)
  runner.rs     — subprocess spawn + NDJSON line reader + mpsc sender
```

**Modified files:**
- `src/cli.rs` — add `-i`/`--interactive` flag; add `graph` subcommand; call `cmd_tui()` and `cmd_graph()`
- `Cargo.toml` — add `ratatui = "0.29"`, `crossterm = "0.28"`

---

## Out of Scope

- Inline resume (provide artifact values in TUI)
- Editing `.line` files in the TUI
- Multi-thruline flowcharts overlaid in a single view
- Run diffing / comparison
