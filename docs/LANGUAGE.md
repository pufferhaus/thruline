# Thruline Language Reference

A `.line` file describes a deterministic multiagent workflow. It contains five kinds of declarations — `import`, `config`, `runner`, `stage`, and `pipeline` — in any order, at the top level.

---

## Comments

```
// this is a comment
```

Single-line only. No block comments.

---

## Identifiers

Names for runners, stages, pipelines, artifacts, and run blocks follow `[A-Za-z][A-Za-z0-9_]*` — letters, digits, and underscores; no hyphens. Model IDs (in `model:` fields) still allow hyphens and dots (e.g. `claude-opus-4-8`).

---

## Imports

```
import "path/to/other.line"
import "shared/runners.line"
```

Resolved relative to the importing file. All declarations merge into the current namespace. Imports deduplicate by canonical path — importing the same file twice is safe. Circular references terminate naturally through path deduplication.

---

## Config

An optional file-level block that sets defaults for the whole pipeline. At most one `config` block is allowed per file (including imported files — duplicate config across files is an error).

```
config {
  model: <model-id>
}
```

| Field | Notes |
|---|---|
| `model` | Default model for any runner that doesn't declare one. Used by the api driver. |

**Example:**

```
config {
  model: claude-sonnet-4-6
}
```

Model resolution order for the **anthropic driver**: `runner.model` → `config.model` → `THRULINE_DEFAULT_MODEL` env var → error.

The stdio driver passes the runner spec as-is to the harness; model resolution is the harness's responsibility.

---

## Runners

A `runner` defines a reusable agent configuration. All fields are optional. Stages reference runners by name; the full runner spec is embedded in each `stage_invoke` event, making pipelines self-contained and portable across harnesses.

```
runner <name> {
  model:       <model-id>
  system:      "<inline text>" | file("<path>")
  tools:       [tool1, tool2]
  temperature: <float>
  max_tokens:  <int>
}
```

| Field | Default | Notes |
|---|---|---|
| `model` | `config.model` or harness default | Model identifier, e.g. `claude-sonnet-4-6` |
| `system` | harness default | Inline text or `file("path")` relative to the `.line` file. When absent, the harness uses its own system prompt. |
| `tools` | `[]` | Tool names granted to this runner |
| `temperature` | — | Sampling temperature |
| `max_tokens` | — | Max output tokens |

> **String literals** do not support escape sequences. A `"` character cannot appear inside an inline string — use `file("path")` for system prompts that contain quotes or special characters.

A runner with no fields is valid — it is a named alias for the harness's defaults:

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

// Inherits model from config block
runner light {
  system: file("prompts/light.md")
}

// All defaults
runner passthrough {}
```

---

## Stages

A `stage` is a single agent invocation unit. All fields are optional.

```
stage <name> {
  in:     <artifact-decl>+
  out:    <artifact-decl>+
  runner: <runner-name>
  prompt: "<inline text>" | file("<path>")
  run <name> { ... }
  ...
}
```

| Field | Notes |
|---|---|
| `runner` | References a declared `runner`. When absent, the stage uses harness defaults. When `run` blocks are present, acts as the default runner for runs that don't declare their own. |
| `in` | Input artifacts consumed by this stage |
| `out` | Output artifacts produced by this stage |
| `prompt` | Task-level prompt passed alongside the system prompt |
| `run` | Named parallel invocation (see below) |

A bare stage is valid:

```
stage checkpoint {}
```

### Artifact Declarations

```
<name>[?] as <kind>[in ["a","b",...]][("<seed-path>")]
```

| Part | Meaning |
|---|---|
| `<name>` | Artifact identifier, scoped as `stage.name` at runtime |
| `?` | Optional — stage runs even if this input is absent |
| `as path` | Disk-persisted artifact; absolute path stored in run state |
| `as value` | In-memory string stored in run state |
| `in ["a","b",...]` | Optional value constraint on `value` outputs — `resume` returns an error if the agent produces a value not in the list |
| `("path")` | Seed path — pre-populates the artifact before the stage runs |

**Examples:**

```
stage interview {
  in:  brief? as path("specs/brief.md")
  out: spec    as path
       verdict as value
  runner: analyst
  prompt: file("prompts/task.md")
}

stage classify {
  out: sentiment as value in ["positive","negative","neutral"]
}

stage notify {
  prompt: "Summarize and notify."
}
```

### Artifact Resolution

When a stage is invoked, the runtime resolves each declared `in:` artifact in this order:

**If the source is explicit** (e.g. `classify.language as value` or `input.code as value`):
- Look up that exact key directly. No fallback.
- `input.x` always resolves to the pipeline input namespace, regardless of history.

**If unqualified** (e.g. `language as value`):
1. **`stage.artifact`** — the stage's own store entry (for retry loops where the stage re-ran)
2. **`<prior>.artifact`** — scan completed stages newest-first; first match wins
3. **`input.artifact`** — pipeline input (last resort default; used only when no stage has produced this artifact)

Pipeline inputs are **seeds, not constants.** Once any stage produces an artifact with the same name, subsequent stages will see the stage's version, not the original input. Use an explicit `input.x` source to pin to the original pipeline input:

```
// analyze gets the most recently produced "code" — from revise if it ran, otherwise from --input
stage analyze {
  in: code as value
}

// analyze always gets the original pipeline input, even if revise produced a newer version
stage analyze {
  in: input.code as value
}
```

This means you can freely pass artifacts between stages without explicit wiring:

```
// classify writes classify.language and classify.complexity
// analyze declares in: language — gets classify.language automatically via history
stage classify {
  out: language   as value
       complexity as value
}

stage analyze {
  in:  language   as value   // resolved from classify via history
       complexity as value
  out: verdict as value
}
```

If two prior stages produced the same artifact name, the most recently completed stage wins.

### Run Blocks

A `run` block declares a named parallel invocation inside a stage. When a stage contains `run` blocks, all runs execute simultaneously. Each run has its own runner, prompt, and outputs. Outputs are scoped to the stage namespace (`stage.artifact`).

```
stage <name> {
  runner: <default-runner>   // optional — fallback for runs without their own runner
  run <name> {
    runner: <runner-name>    // optional — overrides stage default
    prompt: "<text>" | file("<path>")
    out:    <artifact-decl>+
  }
  run <name> { ... }
}
```

The stage-level `runner:` field acts as the default runner for any `run` block that doesn't specify one. A `run` block's `runner:` overrides it.

**Example:**

```
stage dual_review {
  runner: analyst            // default for runs without their own runner
  run quality {
    prompt: "Assess correctness and completeness."
    out: verdict as value
         notes   as value
  }
  run risk {
    runner: critic           // overrides the stage default
    prompt: "Identify risks and failure modes."
    out: risks    as value
         severity as value
  }
}
```

> Each run block receives the same input artifacts as the stage. Resume each run with `thruline resume <run-id> --stage <name> --run <run-name> --artifact key=value`. The stage is complete when all runs have reported back.

---

## Thruline

A `thruline` block declares control flow: which stage runs first, and how outputs route to subsequent stages.

```
thruline <name> {
  start: <stage-name>
  routes {
    <route>
    ...
  }
}
```

### Routes

```
<source> -> <target>[<fan-out>] [max:N]
```

#### Source forms

| Form | Meaning |
|---|---|
| `stage-name` | Unconditional — fires after this stage completes |
| `stage.artifact == "value"` | Predicate — fires if the named ref artifact equals value |
| `stage.artifact != "value"` | Predicate — fires if the named ref artifact does not equal value |
| `stage-name[*]` | Fires after the stage completes (equivalent to unconditional; was "fan-in") |

Routes are evaluated in declaration order; the first match wins. An unconditional route placed last acts as a fallthrough.

#### Target forms

| Form | Meaning |
|---|---|
| `stage-name` | Route to stage; single-agent invocation |
| `stage-name[*]` | Route to stage with **parallel hint** — tells the harness the stage can use subagents; count is harness-decided |
| `stage-name[*N]` | Route to stage with **parallel hint** suggesting up to N subagents |

**Parallel hint semantics:** `[*]` and `[*N]` are instructions to the harness, not directives to the orchestrator. When a route with a parallel spec is taken, the next `stage_invoke` event carries a `parallel` field. The harness reads this and appends subagent instructions to the agent's context. The agent decides internally how to use subagents and returns its outputs as a single stage result. Thruline sees one invocation in, one `stage_complete` out.

This keeps runners atomic (a runner defines a single task) while routes signal where parallelism is appropriate.

#### Per-route visit cap

```
<source> -> <target> [max:N]
```

`[max:N]` sets an optional per-route visit cap on the target stage. When the target stage has been visited `N` or more times via this route, the runtime raises an error instead of routing again. Overrides the global default of 100. Useful for intentional retry loops with a bounded number of attempts.

**Example:**

```
routes {
  review.verdict != "approved" -> draft [max:4]   // at most 4 revision attempts
}
```

`[max:N]` may be combined with fan-out: `a -> b[*3] [max:10]`.

#### Example

```
thruline feature_dev {
  start: interview
  routes {
    interview.verdict == "rejected" -> interview [max:5]   // at most 5 retry attempts
    interview.verdict == "approved" -> review
    review -> implement[*3]                                // fan-out, max 3
    implement[*] -> summarize                              // fan-in
  }
}
```

---

## CLI

```
thruline validate <file.line>                   # Parse and validate
thruline inspect  <file.line>                   # Print pipeline graph and stages
thruline run      <file.line>                   # Run with stdio driver (default)
thruline run      <file.line> --driver anthropic # Run via Anthropic API directly
thruline run      <file.line> --pipeline <name> # Select pipeline when multiple defined
thruline runs                                   # List all runs
thruline status   <run-id>                      # Show run state and artifacts
thruline resume   <run-id> --stage <name> \
                  --artifact key=value \
                  --artifact path=file:///abs/path
```

---

## Execution Model

Thruline uses a **checkpoint-and-resume** model. Two drivers:

### stdio driver (harness mode)

1. `thruline run` emits `pipeline_start` + `stage_invoke` then exits
2. Harness invokes the agent, collects outputs
3. Harness calls `thruline resume` with artifact key=value pairs
4. Runtime evaluates routes and emits the next `stage_invoke` or `pipeline_done`

State persists to `~/.thruline/runs/<run-id>/state.json`, enabling resume after process death.

### anthropic driver (standalone mode)

`thruline run --driver anthropic` calls the Anthropic Messages API directly. Requires `ANTHROPIC_API_KEY`.

Model resolution: `runner.model` → `config.model` → `THRULINE_DEFAULT_MODEL` env var → error.

---

## Event Protocol (NDJSON)

Each event is one JSON line tagged `"event": "<type>"`. Optional fields are omitted.

| Event | When emitted | Key fields |
|---|---|---|
| `pipeline_start` | Run begins | `run_id`, `pipeline` |
| `stage_invoke` | Stage ready | `stage`, `runner`, `artifacts`, `prompt` |
| `stage_complete` | Stage done | `stage`, `outputs` |
| `stage_error` | Stage failed | `stage`, `error` |
| `route_taken` | Route matched | `from`, `to`, `predicate` |
| `parallel_start` | Fan-out begins | `stage`, `count`, `concurrency_limit` |
| `parallel_slot_open` | One slot ready | `stage`, `slot`, `total` |
| `parallel_done` | All slots done | `stage`, `results` |
| `pipeline_done` | Pipeline finished | `outputs` |
| `pipeline_error` | Pipeline failed | `stage`, `error` |

### Runner spec in `stage_invoke`

```json
{
  "name": "analyst",
  "model": "claude-opus-4-8",     // absent if not declared
  "system": "You are an analyst.", // absent if not declared
  "tools": ["read_file"],
  "temperature": 0.7              // absent if not declared
}
```

Stages without `runner:` emit `name: "default"` with no model or system — harness uses its own defaults.

### Agent output protocol

Harnesses should instruct agents to respond with a JSON object:

```json
{
  "verdict": "approved",
  "spec": "file:///tmp/spec.md"
}
```

`file` artifacts: write to disk, return absolute path with `file://` prefix. `ref` artifacts: plain string value. Resume: `thruline resume --artifact key=value`.

---

## Validation

`thruline validate` checks:

- `runner:` references in stages and `run` blocks resolve to declared runners
- Route source/target stages exist
- Predicate artifacts declared as outputs on the referenced stage
- Concurrency limits ≥ 1
- No duplicate runner/stage/thruline names
- At most one `config` block per file

Warnings: stages unreachable from a thruline's `start`.

---

## Feature Gaps

**Compound route predicates**
`==` and `!=` against a single string only. No `&&`, `||`, numeric comparisons.

**Model string unvalidated**
Typos in model identifiers fail at API call time, not validate time.

**No string escape sequences**
Quoted strings terminate at the first `"` — backslash escaping is not supported. Use `file("path")` for system prompts or prompts containing quotes or special characters.

**Value constraints on path outputs not enforced**
`in [...]` is parsed on any `artifact_decl` but only enforced at runtime for `value` outputs. Declaring a constraint on a `path` output is a no-op.

---

## Language Server (LSP)

`thruline lsp` starts a language server (stdio transport, LSP protocol). Configure your editor:

**Neovim** (nvim-lspconfig):
```lua
require('lspconfig').configs.thruline = {
  default_config = {
    cmd = { 'thruline', 'lsp' },
    filetypes = { 'line' },
    root_dir = require('lspconfig.util').find_git_ancestor,
  }
}
require('lspconfig').thruline.setup {}
```

**Zed** — add to `~/.config/zed/settings.json`:
```json
{
  "lsp": {
    "thruline": {
      "binary": { "path": "thruline", "arguments": ["lsp"] }
    }
  }
}
```

Current capabilities: full diagnostics (all validator errors and warnings), keyword completion.

