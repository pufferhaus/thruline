# Thruline Design Spec
_2026-05-30_

## Overview

Thruline is a minimal DSL and runtime for describing deterministic multiagent workflows. A `.tl` file defines runners (agent identity + capability), stages (nodes with typed input/output contracts), and pipelines (routing graphs that wire stages together). The Thruline binary parses, validates, and drives execution — emitting structured events that a driver (harness skill or direct API) acts on.

Pipelines are fully self-contained: runner definitions bundle everything needed to invoke an agent, so no ambient named agents need to exist in the harness at runtime.

---

## Language

### File structure

A `.tl` file contains any mix of:
- `import` directives
- `runner` declarations
- `stage` declarations
- `pipeline` declarations

```
import "runners/workers.tl"
import "stages/review.tl"

runner eng-lead { ... }
stage tip { ... }
pipeline feature-dev { ... }
```

### Imports

```
import "path/to/other.tl"
```

Pulls runner, stage, and pipeline definitions from another `.tl` file. Paths resolve relative to the importing file. Circular imports are a static error.

### Runners

A `runner` defines the full identity and capability of an agent. Stages reference runners by name; the driver constructs the invocation from the runner definition at runtime.

```
runner <name> {
  model:       <model-id>
  system:      file("<path.md>") | "<inline string>"
  [tools:      [<tool-id>, ...]]
  [temperature: <float>]
  [max_tokens: <int>]
}
```

- `model` — required. Model identifier (e.g. `claude-opus-4-8`, `claude-sonnet-4-6`)
- `system` — required. System prompt defining the agent's identity and behavior. `file()` loads from an `.md` file; inline strings for short prompts
- `tools` — optional list of tool names the driver grants this runner
- `temperature` / `max_tokens` — optional inference parameters

Example:

```
runner feature-interviewer {
  model:  claude-opus-4-8
  system: file("prompts/interviewer-system.md")
  tools:  [read_file, write_file]
  temperature: 0.7
}

runner reviewer {
  model:  claude-sonnet-4-6
  system: file("prompts/reviewer-system.md")
}

runner worker {
  model:  claude-sonnet-4-6
  system: "You are a focused implementation agent. Complete the task described in your input artifacts."
  tools:  [read_file, write_file, run_tests]
}
```

### Artifacts

Artifacts are the data that flows between stages. Two types:

| Type | Declared as | Lifecycle |
|------|-------------|-----------|
| `file` | `out: spec as file` | Written to run dir on disk; survives process restart |
| `ref` | `out: verdict as ref` | Held in runtime state; lost if process dies without checkpoint |

A `file` artifact can be seeded from an existing path at pipeline start:

```
in: brief as file("specs/feature-brief.md")
```

### Stage declarations

```
stage <name> {
  in:  <artifact> as <type>[?][("<path>")]
       [<artifact> as <type>[?][("<path>")] ...]
  out: <artifact> as <type>
       [<artifact> as <type> ...]
  agent:  <runner-id>
  [prompt: file("<path>") | "<inline>"]
  [format: <format-id>]
}
```

- `?` marks an optional input — the pipeline can invoke the stage without it
- A stage with no `in` block is a valid entry point
- Multiple `out` lines are allowed; each artifact is independently consumable by downstream stages
- `agent` references a declared `runner` by name — resolved at validate time, error if not found
- `prompt` is an optional task-level prompt injected alongside input artifacts (distinct from the runner's system prompt)
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

**Concurrency limit syntax:** `[*N]` where N is a positive integer caps the number of simultaneously running agents in a fan-out. Without a limit, `[*]` is unbounded.

Multiple routes from the same source are evaluated in declaration order; first match wins. A stage with no matching outbound route halts the pipeline (success if it has `out` artifacts; error otherwise).

### Full example

```
// runners.tl
runner feature-interviewer {
  model:  claude-opus-4-8
  system: file("prompts/interviewer-system.md")
  tools:  [read_file, write_file]
}

runner reviewer {
  model:  claude-sonnet-4-6
  system: file("prompts/reviewer-system.md")
}

runner eng-lead {
  model:  claude-opus-4-8
  system: file("prompts/eng-lead-system.md")
}

runner worker {
  model:       claude-sonnet-4-6
  system:      "You are a focused implementation agent."
  tools:       [read_file, write_file, run_tests]
  temperature: 0.3
}

runner summarizer {
  model:  claude-haiku-4-5
  system: file("prompts/summarizer-system.md")
}
```

```
// feature-dev.tl
import "runners.tl"
import "stages/implement.tl"

stage interview {
  in:  brief? as file("specs/brief.md")
  out: spec    as file
       verdict as ref
  agent:  feature-interviewer
  prompt: file("prompts/interview-task.md")
}

stage review {
  in:  spec as file
  out: verdict as ref
  agent:  reviewer
}

stage tip {
  in:  verdict as ref
  out: tip as file
  agent:  eng-lead
}

stage summary {
  in:  result as ref
  out: notes  as file
  agent:  summarizer
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
  - all runner refs in stages resolve
  - all stage refs in routes resolve
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

`AgentInvocation` carries the full resolved runner spec (model, system prompt content, tools, params) plus the stage's input artifacts and task prompt. Drivers never need to look up agent identity — everything is in the invocation.

---

## Event Protocol

Events are newline-delimited JSON emitted to stdout. All events share a common envelope:

```json
{ "event": "<type>", "run_id": "<id>", "ts": "<iso8601>", ...payload }
```

| Event | Payload fields |
|-------|----------------|
| `pipeline_start` | `pipeline`, `inputs` |
| `stage_invoke` | `stage`, `runner` (full spec), `artifacts`, `prompt?` |
| `stage_complete` | `stage`, `outputs` |
| `stage_error` | `stage`, `error` |
| `route_taken` | `from`, `to`, `predicate` |
| `parallel_start` | `stage`, `count`, `concurrency_limit` |
| `parallel_slot_open` | `stage`, `slot`, `total` |
| `parallel_done` | `stage`, `results` |
| `pipeline_done` | `outputs` |
| `pipeline_error` | `stage`, `error` |

The `runner` field in `stage_invoke` contains the resolved runner definition — model, system prompt text (already loaded from file), tools, and params. The harness skill uses this directly to construct the agent call with no external lookup.

---

## CLI

```
thruline validate <file.tl>
    Parse and statically validate. Exit 0 on success. No run.

thruline inspect <file.tl>
    Print pipeline graph as ASCII DAG. Shows stages, runners, routes, predicates.

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
3. On `stage_invoke` — uses the embedded runner spec to construct and spawn the agent via the harness `Agent` tool
4. Feeds agent output back via `thruline resume`
5. Renders a live pipeline display (text, not TaskCreate)

**Display format:**

```
pipeline: feature-dev  [run: abc123]

  ● interview  (feature-interviewer / opus-4-8)   ✓ complete
  │  verdict: approved, spec: artifacts/spec.md
  ● review     (reviewer / sonnet-4-6)            ⟳ running
  │
  ○ tip        (eng-lead / opus-4-8)              pending
  │
  ○ implement  (worker / sonnet-4-6) [*3]         pending  (parallel, max 3)
  │
  ○ summary    (summarizer / haiku-4-5)           pending
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
    ast.rs              # Runner, Stage, Pipeline, Route AST nodes
    validator.rs        # resolves runners, stages, artifacts; warns on unreachable
    runtime/
      mod.rs
      state.rs
      artifact.rs
      scheduler.rs      # handles [*N] concurrency limits
    driver/
      mod.rs
      stdio.rs
      api.rs
    events.rs
  skill/
    thruline.md         # companion Claude Code skill
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
| All `agent:` refs in stages resolve to a declared runner | Error |
| All stage refs in routes resolve to a declared stage | Error |
| All artifact refs in route predicates exist as stage outputs | Error |
| Parallel fan-in stage exists for every `[*N]` fan-out | Error |
| `[*N]` where N < 1 | Error |
| Circular imports | Error |
| Multiple runners/pipelines/stages with same name in import tree | Error |
| Runner `model` field is empty or missing | Error |
| Runner `system` field is empty or missing | Error |
| `file(...)` paths missing at validation time | Warning |
| Unreachable stages | Warning |

Cycles in the route graph (retry loops) are valid and not flagged as errors.
