# Thruline

**Deterministic multiagent workflows, expressed as code.**

Thruline is a DSL and runtime for defining agent pipelines where every handoff, branch, and artifact is explicit. You describe what each agent produces and how outputs route to the next agent. Thruline runs it, checkpoints state after every step, and resumes from exactly where it stopped.

```
config {
  model: claude-sonnet-4-6
}

runner analyst {
  system: "You are a code reviewer. Respond with JSON."
  temperature: 0.2
}

thruline review {
  inputs {
    code as path
  }
  start: assess
  routes {
    assess.verdict == "approved" -> report
    assess.verdict != "approved" -> revise
    revise -> assess                        // loops back until approved
  }
}

stage assess {
  in:  code    as path
  out: verdict as value
       notes   as value
  runner: analyst
  prompt: "Review the code. Return JSON with verdict (approved/rejected) and notes."
}

stage revise {
  in:  code  as path
       notes as value
  out: code  as path
  runner: analyst
  prompt: "Apply the reviewer notes and return the revised code."
}

stage report {
  in:  notes as value
  out: summary as value
  prompt: "Write a final review summary."
}
```

```bash
# Standalone — calls Anthropic API directly
ANTHROPIC_API_KEY=sk-... thruline run review.line --driver api \
  --input code=file:///path/to/code.rs

# Harness mode — Claude Code drives the agents
thruline run review.line --input code=file:///path/to/code.rs
# → emits stage_invoke; call your agent, then:
thruline resume <run-id> --stage assess \
  --artifact verdict=approved --artifact notes="Looks good."
```

---

## Why Thruline

**Reproducible.** Every run is checkpointed. Kill the process, restart, and it continues from the last completed stage.

**Explicit routing.** Branching, retry loops, and fan-out are declared in the file — no imperative glue code.

**Portable.** The `stage_invoke` event embeds the full runner spec. Any harness that can read NDJSON can drive the agents — Claude Code, a shell script, a CI job.

**Self-contained.** A `.line` file is the full description: agents, prompts, routing, and inputs. No ambient configuration.

---

## How it works

Thruline has two modes:

**Harness mode** (default) — `thruline run` emits a `stage_invoke` JSON event and exits. Your harness (Claude Code, a script, anything) calls the agent and feeds outputs back with `thruline resume`. State is checkpointed after every step to `~/.thruline/runs/`.

**Standalone mode** (`--driver api`) — Calls the Anthropic Messages API directly. No harness needed.

---

## The Language

A `.line` file contains five declaration types.

### `config`
File-level defaults. Sets the fallback model for all runners.

```
config {
  model: claude-sonnet-4-6
}
```

### `runner`
A reusable agent definition. All fields are optional — absent fields inherit from `config` or the harness default.

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
A single agent invocation. Declares what it consumes, what it produces, and how to run it.

```
stage review {
  in:  code     as path
       context? as value     // optional — stage runs even if absent
  out: verdict  as value
       report   as path      // path = disk file; value = in-memory string
  runner: analyst
  prompt: file("prompts/review.md")
}
```

Artifacts flow automatically — if `classify` outputs `language as value`, any later stage can declare `in: language as value` and receive it without explicit wiring.

### `thruline`
The routing declaration: which stage runs first and how outputs branch.

```
thruline feature-dev {
  inputs {
    brief    as path      // required input
    context? as value     // optional input
  }
  start: plan
  routes {
    plan.verdict == "rejected" -> plan     // retry loop
    plan.verdict == "approved" -> implement
    implement -> review[*3]               // parallel hint: up to 3 subagents
    review[*] -> summarize
  }
}
```

Route types: unconditional, `==`/`!=` predicate, fan-out `[*N]`, fan-in `[*]`.

### `import`
Pull in runners or stages from another file.

```
import "shared/runners.line"
```

---

## CLI

```bash
thruline validate <file.line>              # Parse and validate
thruline inspect  <file.line>              # Show routing graph and stages
thruline run      <file.line>              # Run (stdio/harness mode)
thruline run      <file.line> \
  --driver api                             # Run standalone (Anthropic API)
  --pipeline <name>                        # Select thruline by name
  --input key=value                        # Set input artifact
  --input file=file:///abs/path
thruline runs                              # List all runs
thruline status   <run-id>                 # Show run state
thruline resume   <run-id> \
  --stage <name>                           # Feed agent output back
  --artifact key=value
```

---

## Install

```bash
# Homebrew
brew tap pufferhaus/tap
brew install thruline

# From source (requires Rust 1.70+)
cargo install --git https://github.com/pufferhaus/thruline
```

---

## Examples

| Example | What it shows |
|---|---|
| [`examples/sentiment/`](examples/sentiment/) | Predicate routing, two runners, pipeline inputs |
| [`examples/code-review/`](examples/code-review/) | Retry loop, pass/fail routing, two-stage revision |

```bash
# Sentiment analysis
ANTHROPIC_API_KEY=sk-... thruline run examples/sentiment/pipeline.line \
  --driver api --input text="I love Rust!"

# Code review with retry loop
ANTHROPIC_API_KEY=sk-... thruline run examples/code-review/review.line \
  --driver api --input code=file:///path/to/file.rs
```

---

## Events (NDJSON)

Thruline communicates via NDJSON on stdout:

```json
{"event":"pipeline_start","run_id":"a1b2c3d4","pipeline":"review","ts":"..."}
{"event":"stage_invoke","stage":"assess","runner":{"name":"analyst","model":"claude-opus-4-8"},"artifacts":{"code":"..."},"outputs":[{"name":"verdict","kind":"value"}]}
{"event":"route_taken","from":"assess","to":"report","predicate":"assess.verdict == \"approved\""}
{"event":"pipeline_done","run_id":"a1b2c3d4","outputs":{"assess.verdict":"approved","report.summary":"..."}}
```

The `stage_invoke` event embeds the full runner spec — harnesses don't need ambient lookup.

---

## Status

Core execution is complete: sequential stages, predicate routing, retry loops, pipeline inputs, checkpoint/resume, and the API driver.

Two features are parsed and validated but not yet wired in the runtime:
- **Parallel fan-out** (`[*N]`/`[*]`) — hint passes to harness; full scheduler pending
- **`run` blocks** (named parallel invocations within a stage)

See [`docs/LANGUAGE.md`](docs/LANGUAGE.md) for the full language reference.

[thruline.work](https://thruline.work)
