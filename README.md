# Thruline

A minimal DSL and runtime for deterministic multiagent pipelines.

You describe a workflow in a `.line` file — agents, what they produce, and how outputs route between them. Thruline runs it, checkpoints state after every agent call, and can resume if the process dies.

```
config {
  model: claude-sonnet-4-6
}

runner analyst {
  system: file("prompts/analyst.md")
  temperature: 0.3
}

stage classify {
  out: language   as value
       complexity as value
  runner: analyst
  prompt: "Classify the code snippet."
}

stage review {
  in:  classify.language   as value
       classify.complexity as value
  out: verdict as value
       notes   as value
  runner: analyst
  prompt: "Review code quality."
}

stage report {
  in:  verdict as value
  runner: analyst
  out: summary as value
}

pipeline code-review {
  inputs {
    code as value
  }
  start: classify
  routes {
    classify                        -> review
    review.verdict == "approved"    -> report
    review.verdict != "approved"    -> review   // retry
  }
}
```

```
thruline run pipeline.line --input code="def add(a, b): return a + b"
```

---

## How it works

Thruline has two execution modes:

**Harness mode** (default, `--driver stdio`) — a harness such as Claude Code drives agent invocations. `thruline run` emits a `stage_invoke` NDJSON event and exits. The harness calls the agent, then calls `thruline resume` with the outputs. The runtime evaluates routes and emits the next event. State is checkpointed to `~/.thruline/runs/` after every step.

**Standalone mode** (`--driver api`) — `thruline run --driver api` calls the Anthropic Messages API directly with `ANTHROPIC_API_KEY`. No external harness required.

---

## Language

A `.line` file contains five declaration types:

### `config`
File-level defaults. Sets the fallback model for runners that don't declare one.

```
config {
  model: claude-sonnet-4-6
}
```

### `runner`
A reusable agent configuration. All fields optional — absent fields defer to the harness or `config`.

```
runner analyst {
  model:       claude-opus-4-8
  system:      file("prompts/analyst.md")
  tools:       [read_file, write_file]
  temperature: 0.7
  max_tokens:  8192
}
```

### `stage`
A single agent invocation. Declares inputs, outputs, which runner to use, and an optional task prompt. Stages with `run` blocks invoke multiple runners simultaneously (parallel execution coming soon).

```
stage review {
  in:  code     as value
       context? as value      // optional — stage runs even if absent
  out: verdict  as value
       report   as path       // path = disk file; value = in-memory string
  runner: analyst
  prompt: file("prompts/review.md")
}
```

**Artifact resolution** for `in:` declarations:
1. `stage.name` — the stage's own prior output (e.g. after a retry)
2. `input.name` — pipeline input supplied via `--input`
3. History scan — prior stages newest-first; first that produced an artifact with this name wins
4. Explicit source — `classify.language as value` looks up `classify.language` directly, bypassing history

### `pipeline`
Declares inputs and routing logic.

```
pipeline feature-dev {
  inputs {
    brief    as path     // required
    context? as value    // optional
  }
  start: interview
  routes {
    interview.verdict == "rejected" -> interview   // retry loop
    interview.verdict == "approved" -> implement
    implement -> implement[*3]                     // fan-out (coming soon)
    implement[*] -> summarize                      // fan-in
  }
}
```

Route types: unconditional, `==` predicate, `!=` predicate, fan-out `[*N]`, fan-in `[*]`.

### `import`
Merge declarations from another `.line` file. Useful for shared runner libraries.

```
import "shared/runners.line"
```

---

## CLI

```
thruline validate <file.line>              # Parse and validate
thruline inspect  <file.line>              # Print pipeline graph
thruline run      <file.line>              # Run (stdio driver)
thruline run      <file.line> \
  --driver api                             # Run standalone (Anthropic API)
  --pipeline <name>                        # Select pipeline by name
  --input key=value                        # Set pipeline input (repeatable)
  --input file=file:///abs/path
thruline runs                              # List all runs
thruline status   <run-id>                 # Show run state and artifacts
thruline resume   <run-id> \
  --stage <name>                           # Feed agent output back
  --artifact key=value
  --artifact file=file:///abs/path
```

---

## Events (NDJSON)

Each event is one JSON line tagged `"event": "<type>"`. Optional fields are omitted when absent.

```json
{"event":"pipeline_start","run_id":"a1b2c3d4","pipeline":"code-review"}
{"event":"stage_invoke","stage":"classify","runner":{"name":"analyst","model":"claude-opus-4-8","system":"..."},"artifacts":{"code":"..."}}
{"event":"route_taken","from":"classify","to":"review","predicate":"Stage(\"classify\")"}
{"event":"pipeline_done","outputs":[]}
```

The `stage_invoke` event embeds the full runner spec so harnesses don't need ambient agent lookup — pipelines are self-contained.

---

## Install

```
git clone https://github.com/pufferhaus/thruline
cd thruline
cargo install --path .
```

Requires Rust 1.70+.

---

## Status

Core pipeline execution (sequential, predicate routing, retry loops, pipeline inputs) is complete and working. Two features are implemented at the language level but not yet wired in the runtime:

- **Parallel fan-out/fan-in** (`[*N]` / `[*]`) — Scheduler infrastructure exists, runtime wiring pending
- **`run` blocks** (parallel stage invocations) — same parallel execution path

See [`docs/LANGUAGE.md`](docs/LANGUAGE.md) for the full language reference.

---

## Domain

[thruline.work](https://thruline.work)
