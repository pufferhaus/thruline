# Thruline Language Reference

A `.line` file describes a deterministic multiagent workflow. It contains four kinds of declarations — `import`, `runner`, `stage`, and `pipeline` — in any order, at the top level.

---

## Comments

```
// this is a comment
```

Single-line only. No block comments.

---

## Imports

```
import "path/to/other.line"
import "shared/runners.line"
```

Imports are resolved relative to the importing file. All declarations in the imported file are merged into the current namespace. Imports deduplicate by canonical path — importing the same file twice is safe. Circular imports are detected and rejected.

---

## Runners

A `runner` defines a reusable agent configuration. All fields are optional. Stages reference runners by name; the full runner spec is embedded in each `stage_invoke` event, making pipelines self-contained and portable across harnesses.

```
runner <name> {
  model:       <model-id>
  system:      "<inline prompt>" | file("<path>")
  tools:       [tool1, tool2]
  temperature: <float>
  max_tokens:  <int>
}
```

| Field | Type | Default | Notes |
|---|---|---|---|
| `model` | string | harness default | Model identifier, e.g. `claude-sonnet-4-6`. When absent, the harness or driver picks the model. |
| `system` | string or file ref | harness default | Inline text or `file("path/to/prompt.md")` relative to the `.line` file. When absent, the harness uses its own system prompt. |
| `tools` | list of identifiers | `[]` | Tool names granted to this runner. |
| `temperature` | float | — | Sampling temperature. |
| `max_tokens` | int | — | Max output tokens. |

A runner with no fields is valid — it acts as a named alias for the harness's defaults:

```
runner default {}
```

**Examples:**

```
// Fully specified
runner analyst {
  model: claude-opus-4-8
  system: file("prompts/analyst.md")
  tools: [read_file, write_file]
  temperature: 0.7
  max_tokens: 8192
}

// Minimal — harness picks model and system prompt
runner light {}
```

---

## Stages

A `stage` is a single agent invocation unit. All fields are optional.

```
stage <name> {
  in:     <artifact-decl>+
  out:    <artifact-decl>+
  runner: <runner-name>
  prompt: "<inline prompt>" | file("<path>")
  format: <identifier>
}
```

| Field | Required | Notes |
|---|---|---|
| `runner` | no | References a declared `runner`. When absent, the stage uses harness defaults — no model or system prompt override. |
| `in` | no | Input artifacts consumed by this stage. |
| `out` | no | Output artifacts produced by this stage. |
| `prompt` | no | Task-level prompt passed alongside the system prompt. |
| `format` | no | Output format hint (currently parsed but not enforced — see Feature Gaps). |

A bare stage with no fields is valid:

```
stage checkpoint {}
```

### Artifact Declarations

```
<name>[?] as <kind>[("<seed-path>")]
```

| Part | Meaning |
|---|---|
| `<name>` | Artifact identifier, scoped as `stage.name` at runtime |
| `?` | Optional — stage runs even if this input is absent |
| `as file` | Disk-persisted artifact; absolute path stored in run state |
| `as ref` | In-memory string value stored in run state |
| `("path")` | Seed path — pre-populates the artifact before the stage runs |

**Examples:**

```
stage interview {
  in:  brief? as file("specs/brief.md")
  out: spec    as file
       verdict as ref
  runner: analyst
  prompt: file("prompts/task.md")
}

stage notify {
  // no runner: harness drives this with its own defaults
  prompt: "Summarize results and notify the team."
}
```

---

## Pipelines

A `pipeline` declares control flow: which stage runs first, and how outputs route to subsequent stages.

```
pipeline <name> {
  start: <stage-name>
  routes {
    <route>
    ...
  }
}
```

### Routes

Each route is one line:

```
<source> -> <target>[<fan-out>] [parallel]
```

#### Source forms

| Form | Meaning |
|---|---|
| `stage-name` | Unconditional — fires after this stage completes |
| `stage.artifact == "value"` | Predicate — fires if the named ref artifact equals the value |
| `stage.artifact != "value"` | Predicate — fires if the named ref artifact does not equal the value |
| `stage-name[*]` | Fan-in — fires after all parallel slots of this stage complete |

Routes are evaluated in declaration order; the first match wins. An unconditional route placed last acts as a default/fallthrough.

#### Target forms

| Form | Meaning |
|---|---|
| `stage-name` | Single next stage |
| `stage-name[*]` | Fan-out — unlimited parallel copies |
| `stage-name[*N]` | Fan-out — max N concurrent copies |

The `parallel` keyword is required on any route targeting a `[*N]` fan-out.

#### Examples

```
pipeline feature-dev {
  start: interview
  routes {
    // Retry loop — back-edge
    interview.verdict == "rejected" -> interview

    // Conditional advance
    interview.verdict == "approved" -> review

    // Fan-out with concurrency limit
    review -> implement[*3] parallel

    // Fan-in: wait for all implement slots
    implement[*] -> summarize
  }
}

pipeline linear {
  start: gather
  routes {
    gather -> analyze
    analyze -> report
  }
}
```

---

## CLI

```
thruline validate <file.line>                   # Parse and validate
thruline inspect  <file.line>                   # Print pipeline graph and stages
thruline run      <file.line>                   # Run with stdio driver (default)
thruline run      <file.line> --driver api      # Run with direct API driver
thruline run      <file.line> --pipeline <name> # Select pipeline by name
thruline runs                                   # List all runs
thruline status   <run-id>                      # Show run state and artifacts
thruline resume   <run-id> --stage <name> \
                  --artifact key=value \
                  --artifact path=file:///abs/path/to/file
```

---

## Execution Model

Thruline uses a **checkpoint-and-resume** model. Two drivers are available:

### stdio driver (harness mode)

A harness (e.g. Claude Code) drives execution:

1. `thruline run` emits a `pipeline_start` event then a `stage_invoke` event and exits
2. Harness reads the event, invokes the agent, collects outputs
3. Harness calls `thruline resume` with artifact key=value pairs
4. Runtime evaluates routes and emits the next `stage_invoke`, or `pipeline_done`

State is persisted to `~/.thruline/runs/<run-id>/state.json` between steps, enabling resume after process death.

### api driver (standalone mode)

`thruline run --driver api` calls the Anthropic Messages API directly, driving the full pipeline without an external harness. Requires `ANTHROPIC_API_KEY`. When a stage's runner has no `model`, the api driver defaults to `claude-sonnet-4-6`.

---

## Event Protocol (NDJSON)

Each event is one JSON line on stdout, tagged with `"event": "<type>"`. Optional fields are omitted when absent.

| Event | When emitted | Key fields |
|---|---|---|
| `pipeline_start` | Run begins | `run_id`, `pipeline` |
| `stage_invoke` | Stage ready to execute | `stage`, `runner` (full spec), `artifacts`, `prompt` |
| `stage_complete` | Stage finished | `stage`, `outputs` |
| `stage_error` | Stage failed | `stage`, `error` |
| `route_taken` | A route matched | `from`, `to`, `predicate` |
| `parallel_start` | Fan-out begins | `stage`, `count`, `concurrency_limit` |
| `parallel_slot_open` | One slot ready | `stage`, `slot`, `total` |
| `parallel_done` | All slots complete | `stage`, `results` |
| `pipeline_done` | Pipeline finished | `outputs` (file artifact paths) |
| `pipeline_error` | Pipeline failed | `stage`, `error` |

### `stage_invoke` runner spec

The `runner` field in `stage_invoke` contains:

```json
{
  "name": "analyst",
  "model": "claude-opus-4-8",      // absent if not declared
  "system": "You are an analyst.",  // absent if not declared
  "tools": ["read_file"],
  "temperature": 0.7               // absent if not declared
}
```

When `model` or `system` is absent, the harness uses its own defaults. Stages without a `runner:` declaration emit a minimal spec (`name: "default"`, no model or system) — the harness drives the agent entirely with its own configuration.

### Agent output protocol

When driving a stage, the harness should instruct the agent to respond with a JSON object mapping output artifact names to values:

```json
{
  "verdict": "approved",
  "spec": "file:///tmp/spec.md"
}
```

For `file` artifacts: write the file to disk, return the absolute path prefixed with `file://`. For `ref` artifacts: return the value as a plain string. Pass this back via `thruline resume --artifact key=value`.

---

## Validation

`thruline validate` checks:

- All `runner:` references in stages resolve to a declared `runner`
- All route source/target stages exist
- All predicate artifacts are declared as outputs on the referenced stage
- Every `[*N]` fan-out has a matching `[*]` fan-in in the same pipeline
- Concurrency limits are ≥ 1
- No duplicate runner/stage/pipeline names

Warnings (non-fatal):
- Stages declared but not reachable from a pipeline's `start`

---

## Feature Gaps

The following are defined in the grammar or AST but not yet fully implemented:

**Parallel fan-out/fan-in** (`[*N]` / `[*]`)
Grammar, AST, validator, and `Scheduler` struct all exist. At runtime, fan-out routes fall through to single sequential execution — the `Scheduler` is not wired into `resume_stage`. See `src/runtime/mod.rs` TODO.

**`format` field on stages**
Parsed and stored in `StageDecl.format` but never acted on. No runtime semantics defined.

**Compound route predicates**
Routes support `==` and `!=` against a single string value only. No `&&`, `||`, numeric comparisons, or range checks.

**No pipeline-level inputs**
No syntax to pass named inputs at `thruline run` time. Artifacts can only be pre-seeded via `seed_init` in stage declarations. There is no equivalent of function parameters for a pipeline.

**No retry limit**
Back-edge routes create unbounded loops. No `max_retries`, `timeout`, or iteration cap.

**`system: file(...)` not validated at validate time**
A `file("nonexistent.md")` system prompt passes validation and only fails at runtime when read.

**Model string unvalidated**
Any non-empty string is accepted as a model identifier. Typos produce runtime API errors, not static validation errors.

**Identifier syntax allows `-`**
Identifiers accept hyphens (`my-runner`, `feature-dev`). This can cause friction in contexts where artifact keys are used as JSON object keys, since hyphenated keys require quoting in most languages.
