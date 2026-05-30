# Thruline Design Spec
_2026-05-30_

## Overview

Thruline is a minimal DSL and runtime for describing deterministic multiagent workflows. A `.tl` file defines stages (agent nodes with typed input/output contracts) and pipelines (routing graphs that wire stages together). The Thruline binary parses, validates, and drives execution — emitting structured events that a driver (harness skill or direct API) acts on.

---

## Language

### File structure

A `.tl` file contains any mix of:
- `import` directives
- `stage` declarations
- `pipeline` declarations

```
import "stages/workers.tl"
import "pipelines/shared.tl"

stage interview { ... }
pipeline feature-dev { ... }
```

### Imports

```
import "path/to/other.tl"
```

Pulls stage and pipeline definitions from another `.tl` file. Paths resolve relative to the importing file. Circular imports are a static error.

### Artifacts

Artifacts are the data that flows between stages. Two types:

| Type | Declared as | Lifecycle |
|------|-------------|-----------|
| `file` | `out: spec as file` | Written to run dir on disk; survives process restart |
| `ref` | `out: verdict as ref` | Held in runtime state; lost if process dies without checkpoint |

A `file` artifact can be seeded from an existing path:

```
in: brief as file("specs/feature-brief.md")
```

An `.md` file can be used as a prompt source for a stage:

```
agent: feature-interviewer
prompt: file("prompts/interview.md")
```

### Stage declarations

```
stage <name> {
  in:  <artifact> as <type>[?][("<path>")]
       [<artifact> as <type>[?][("<path>")] ...]
  out: <artifact> as <type>
       [<artifact> as <type> ...]
  agent: <agent-id>
  [prompt: file("<path>") | "<inline>"]
  [format: <format-id>]
}
```

- `?` marks an optional input — the pipeline can invoke the stage without it
- A stage with no `in` block is a valid entry point
- Multiple `out` lines are allowed; each artifact is independently consumable by downstream stages
- `agent` names the agent identity to invoke; the driver resolves this to an actual model/prompt
- `format` constrains output shape (e.g. `release-notes`, `json-schema`)

### Pipeline declarations

```
pipeline <name> {
  start: <stage>

  routes {
    <route>
    [<route> ...]
  }
}
```

A `route` is one of:

```
// Unconditional
<stage> -> <stage>

// Conditional (evaluates a ref value)
<stage>.<artifact> == "<value>" -> <stage>
<stage>.<artifact> != "<value>" -> <stage>

// Retry / cycle
<stage>.<artifact> == "rejected" -> <stage>   // back-edge is valid

// Parallel fan-out (unbounded)
<stage> -> <target>[*] parallel

// Parallel fan-out with concurrency limit
<stage> -> <target>[*N] parallel              // max N concurrent agents

// Parallel fan-in (implicit — collects all [*] results)
<target>[*] -> <stage>
```

**Concurrency limit syntax:** `[*N]` where N is a positive integer caps the number of simultaneously running agents in a fan-out. Without a limit, `[*]` is unbounded. Examples:

```
tip -> implement[*]   parallel    // unlimited concurrency
tip -> implement[*2]  parallel    // max 2 agents at a time
tip -> implement[*10] parallel    // max 10 agents at a time
```

Multiple routes from the same source are evaluated in declaration order; first match wins. A stage with no matching outbound route halts the pipeline (success if it has `out` artifacts; error otherwise).

### Full example

```
import "stages/workers.tl"

stage interview {
  in:  brief? as file("specs/brief.md")
  out: spec    as file
       verdict as ref
  agent: feature-interviewer
  prompt: file("prompts/interview.md")
}

stage review {
  in:  spec as file
  out: verdict as ref
  agent: reviewer
}

stage tip {
  in:  verdict as ref
  out: tip as file
  agent: eng-lead
}

stage summary {
  in:  result as ref
  out: notes  as file
  format: release-notes
}

pipeline feature-dev {
  start: interview

  routes {
    interview.verdict == "approved" -> review
    interview.verdict == "rejected" -> interview    // retry

    review -> tip

    tip -> implement[*3] parallel                   // max 3 concurrent
    implement[*] -> summary
  }
}
```

---

## Architecture

```
.tl source file(s)
       |
Parser (pest grammar) -> AST
       |
Validator
  - all stage refs resolve
  - artifact type consistency
  - no unresolvable cycles
  - unreachable stage warnings
       |
Runtime / State Machine
  - tracks current stage(s)
  - artifact store (file + ref)
  - evaluates route predicates
  - manages parallel fan-out/fan-in
  - enforces [*N] concurrency limits
  - checkpoints state to run dir
  - emits structured events (NDJSON)
       |
      / \
api      stdio driver
driver   events -> stdout
         harness invokes agents
         results back via `resume`
```

### Driver interface (Rust trait)

```rust
trait ThrulineDriver {
    async fn invoke_agent(&self, invocation: AgentInvocation) -> AgentResult;
    fn emit_event(&self, event: ThrulineEvent);
}
```

The runtime calls `invoke_agent` and `emit_event`; drivers implement both. The stdio driver emits the invocation as an event and blocks waiting for a `resume` command. The API driver calls the Anthropic API directly.

---

## Event Protocol

Events are newline-delimited JSON emitted to stdout. All events share a common envelope:

```json
{ "event": "<type>", "run_id": "<id>", "ts": "<iso8601>", ...payload }
```

| Event | Payload fields |
|-------|----------------|
| `pipeline_start` | `pipeline`, `inputs` |
| `stage_invoke` | `stage`, `agent`, `artifacts` |
| `stage_complete` | `stage`, `outputs` |
| `stage_error` | `stage`, `error` |
| `route_taken` | `from`, `to`, `predicate` |
| `parallel_start` | `stage`, `count`, `concurrency_limit` |
| `parallel_slot_open` | `stage`, `slot`, `total` |
| `parallel_done` | `stage`, `results` |
| `pipeline_done` | `outputs` |
| `pipeline_error` | `stage`, `error` |

---

## CLI

```
thruline validate <file.tl>
    Parse and statically validate. Exit 0 on success. No run.

thruline inspect <file.tl>
    Print pipeline graph as ASCII DAG. Shows stages, routes, predicates.

thruline run <file.tl> [--pipeline <name>] [--driver stdio|api]
    Execute pipeline. Default driver: stdio.
    --pipeline required if file defines multiple pipelines.

thruline resume <run-id> --stage <name> --artifact <key>=<value> [...]
    Feed agent output back to a suspended stdio-driver run.
    <value> is a literal for refs, or a file path for file artifacts.

thruline status <run-id>
    Print current state, completed stages, pending stages, artifact locations.

thruline runs
    List all runs (id, pipeline, status, timestamp).
```

---

## Run State

Each run is persisted to `~/.thruline/runs/<run-id>/`:

```
~/.thruline/runs/abc123/
  state.json          # current stage, artifact store, route history
  artifacts/          # file artifacts written here unless path is absolute
    spec.md
    tip.md
    notes.md
  events.ndjson       # full event log for the run
```

This enables `resume` after process death and `status` queries at any time.

---

## Companion Skill

Ships in the thruline repo at `skill/thruline.md`. Consumers install it into their Claude Code plugin config.

**What it does:**
1. Invokes `thruline run <file.tl> --driver stdio`
2. Reads NDJSON events from the subprocess stdout
3. On `stage_invoke` — spawns the named agent via the harness `Agent` tool
4. Feeds agent output back via `thruline resume`
5. Renders a live pipeline display (text, not TaskCreate)

**Display format:**

```
pipeline: feature-dev  [run: abc123]

  ● interview         ✓ complete
  │  verdict: approved, spec: artifacts/spec.md
  ● review            ⟳ running
  │
  ○ tip               pending
  │
  ○ implement[*3]     pending  (parallel, max 3)
  │
  ○ summary           pending
```

Rendered as plain text output from the skill, refreshed on each event. The binary owns all state; the skill is a pure display + invocation adapter.

---

## Repository Layout

```
thruline/
  src/
    main.rs
    cli.rs
    parser/
      mod.rs
      grammar.pest
    ast.rs
    validator.rs
    runtime/
      mod.rs
      state.rs
      artifact.rs
      scheduler.rs        # handles [*N] concurrency limits
    driver/
      mod.rs
      stdio.rs
      api.rs
    events.rs
  skill/
    thruline.md           # companion Claude Code skill
  docs/
    superpowers/specs/
      2026-05-30-thruline-design.md
  Cargo.toml
  .gitignore
```

---

## Static Validation Rules

| Check | Severity |
|-------|----------|
| All stage refs in routes resolve to a declared stage | Error |
| All artifact refs in route predicates exist as stage outputs | Error |
| Parallel fan-in stage exists for every `[*N]` fan-out | Error |
| `[*N]` where N < 1 | Error |
| Circular imports | Error |
| Multiple pipelines/stages with same name in import tree | Error |
| `file(...)` paths missing at validation time | Warning |
| Unreachable stages | Warning |

Cycles in the route graph (retry loops) are valid and not flagged as errors.
