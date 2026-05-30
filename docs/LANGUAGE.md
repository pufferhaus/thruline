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
import "prompts/runners.line"
```

Imports are resolved relative to the importing file. All declarations in the imported file are merged into the current namespace before validation. Imports deduplicate by canonical path — importing the same file twice is safe. Circular imports are detected and rejected.

---

## Runners

A `runner` is a reusable agent definition. Stages reference runners by name. Because the full runner spec is embedded in each `stage_invoke` event, pipelines are self-contained and portable across harnesses.

```
runner <name> {
  model:       <model-id>
  system:      "<inline prompt>" | file("<path>")
  tools:       [tool1, tool2]
  temperature: <float>
  max_tokens:  <int>
}
```

| Field | Required | Type | Notes |
|---|---|---|---|
| `model` | yes | string | Model identifier, e.g. `claude-sonnet-4-6` |
| `system` | yes | string or file ref | Inline text or `file("path/to/prompt.md")` relative to the `.line` file |
| `tools` | no | list of identifiers | Tool names granted to this runner |
| `temperature` | no | float | Sampling temperature |
| `max_tokens` | no | int | Max output tokens |

**Example:**

```
runner interviewer {
  model: claude-opus-4-8
  system: file("prompts/interviewer.md")
  tools: [read_file, write_file]
  temperature: 0.7
  max_tokens: 8192
}
```

---

## Stages

A `stage` is a single agent invocation unit. It declares what artifacts it consumes, what it produces, and which runner executes it.

```
stage <name> {
  in:     <artifact-decl>+
  out:    <artifact-decl>+
  agent:  <runner-name>
  prompt: "<inline prompt>" | file("<path>")
  format: <identifier>
}
```

| Field | Required | Notes |
|---|---|---|
| `agent` | yes | Must reference a declared `runner` |
| `in` | no | Input artifacts consumed by this stage |
| `out` | no | Output artifacts produced by this stage |
| `prompt` | no | Task-level prompt, passed alongside the system prompt |
| `format` | no | Output format hint (see Feature Gaps) |

### Artifact Declarations

```
<name>[?] as <kind>[("<seed-path>")]
```

| Part | Meaning |
|---|---|
| `<name>` | Artifact identifier, scoped as `stage.name` at runtime |
| `?` | Optional — stage may run even if this input is absent |
| `as file` | Disk-persisted artifact; path stored in run state |
| `as ref` | In-memory string value; stored in run state |
| `("path")` | Seed path — pre-populates the artifact before the stage runs |

**Examples:**

```
stage interview {
  in:  brief? as file("specs/brief.md")
  out: spec    as file
       verdict as ref
  agent:  feature-interviewer
  prompt: file("prompts/task.md")
}
```

```
stage summarize {
  in:  notes as ref
  out: report as file
  agent: summarizer
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
| `stage.artifact == "value"` | Predicate — fires if artifact equals value |
| `stage.artifact != "value"` | Predicate — fires if artifact does not equal value |
| `stage-name[*]` | Fan-in — fires after all parallel slots of this stage complete |

Routes are evaluated in declaration order; the first match wins.

#### Target forms

| Form | Meaning |
|---|---|
| `stage-name` | Single next stage |
| `stage-name[*]` | Fan-out — unlimited parallel copies |
| `stage-name[*N]` | Fan-out — max N concurrent copies |

The `parallel` keyword marks a route as explicitly parallel (required alongside `[*N]` targets).

#### Examples

```
pipeline feature-dev {
  start: interview
  routes {
    // Conditional retry loop
    interview.verdict == "rejected" -> interview

    // Advance on approval
    interview.verdict == "approved" -> review

    // Unconditional fan-out with concurrency limit
    review -> implement[*3] parallel

    // Fan-in: wait for all implement slots, then summarize
    implement[*] -> summarize
  }
}
```

```
pipeline simple {
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
thruline validate <file.line>              # Parse and validate
thruline inspect  <file.line>              # Print pipeline graph
thruline run      <file.line>              # Run with stdio driver (default)
thruline run      <file.line> --driver api # Run with API driver
thruline run      <file.line> --pipeline <name>  # Select pipeline by name
thruline runs                              # List all runs
thruline status   <run-id>                 # Show run state
thruline resume   <run-id> --stage <name> \
                  --artifact key=value \
                  --artifact path=file:///abs/path/to/file
```

---

## Execution Model

Thruline uses a **checkpoint-and-resume** model with two drivers:

### stdio driver (harness mode)

The harness (e.g. Claude Code) drives execution:

1. `thruline run` emits NDJSON events to stdout and exits after `stage_invoke`
2. Harness reads the event, calls the agent, collects output
3. Harness calls `thruline resume` with artifact outputs
4. Runtime evaluates routes and either emits the next `stage_invoke` or `pipeline_done`

### api driver (standalone mode)

`thruline run --driver api` calls the Anthropic Messages API directly and drives the full pipeline without a harness. Requires `ANTHROPIC_API_KEY` in the environment.

---

## Event Protocol (NDJSON)

Each event is one JSON line on stdout, tagged with `"event": "<type>"`.

| Event | When emitted |
|---|---|
| `pipeline_start` | Run begins |
| `stage_invoke` | Stage is about to execute; includes full `RunnerSpec` |
| `stage_complete` | Stage finished; includes output artifact values |
| `stage_error` | Stage failed |
| `route_taken` | A route was matched and followed |
| `parallel_start` | Fan-out begins; includes count and concurrency limit |
| `parallel_slot_open` | One slot of a fan-out is ready |
| `parallel_done` | All fan-out slots complete |
| `pipeline_done` | Pipeline finished; includes output file paths |
| `pipeline_error` | Pipeline failed |

The `stage_invoke` event embeds the full `RunnerSpec` (model, resolved system prompt text, tools, temperature, max_tokens) so harnesses need no ambient agent lookup.

---

## Validation

`thruline validate` checks:

- All `agent:` references resolve to a declared `runner`
- All route source/target stages exist
- All predicate artifacts are declared as outputs on the referenced stage
- Every `[*N]` fan-out has a matching `[*]` fan-in in the same pipeline
- Concurrency limits are ≥ 1
- No duplicate runner/stage/pipeline names
- Runners have non-empty `model` and `system`

Warnings (non-fatal):
- Stages declared but not reachable from any pipeline's `start`

---

## Feature Gaps

The following are defined in the grammar or AST but not yet fully implemented:

**Parallel fan-out/fan-in** (`[*N]` / `[*]`)
Grammar, AST, validator, and `Scheduler` struct all exist. At runtime, fan-out routes currently advance to single sequential execution — the `Scheduler` is not yet wired into `resume_stage`. See `src/runtime/mod.rs` TODO.

**`format` field on stages**
Parsed and stored in `StageDecl.format` but never acted on. Semantics are undefined — intended as an output format hint (e.g. `format: json`) but no runtime behaviour is attached.

**Compound route predicates**
Routes support `==` and `!=` against a single string value. No `&&`, `||`, or numeric comparisons. Multi-condition routing requires multiple stages or encoding into a single artifact value.

**No pipeline-level inputs**
No syntax to pass named inputs at `thruline run` time. Artifacts can only be pre-seeded via `seed_init` in stage declarations (`brief? as file("path")`). There is no equivalent of function arguments for a pipeline.

**No retry limit**
Back-edge routes (`a.verdict == "retry" -> a`) create unbounded loops. No `max_retries`, `timeout`, or iteration cap exists.

**`system: file(...)` not validated at validate time**
A `file("nonexistent.md")` system prompt passes `thruline validate` and only fails at runtime when the path is read. Only empty inline strings are caught statically.

**Model string unvalidated**
Any non-empty string is accepted for `model:`. Typos produce runtime API errors, not static validation errors.

**Identifier syntax allows `-`**
Identifiers accept hyphens (`my-runner`, `feature-dev`). This can cause friction when artifact keys are used as JSON object keys from agent output, since hyphenated keys require quoting in most languages.
