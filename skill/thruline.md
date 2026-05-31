---
name: thruline
description: Run a .line file using the thruline binary as orchestrator. Drives agent invocations through this Claude Code harness and renders live status. Use when asked to run a thruline workflow or .line file.
---

# Thruline Skill

Runs a `.line` file using the `thruline` binary as the orchestrator.
This skill bridges the binary's stdio event protocol with Claude Code's Agent tool.

## Usage

Invoke with a path to a `.line` file:
```
/thruline path/to/workflow.line [--pipeline <name>]
```

## How It Works

1. Run `thruline run <file.line> --driver stdio` as a subprocess
2. Parse `pipeline_start` — create one TaskCreate entry per stage in the pipeline
3. Read subsequent NDJSON events and update the corresponding task via TaskUpdate as each stage changes state
4. On `stage_invoke` — set the task to `in_progress`, then spawn the stage's agent using the Agent tool
5. Call `thruline resume <run-id> --stage <name> --artifact key=value [...]` with the agent's output
6. On route advance — update the task that just completed and the next one to `in_progress`
7. Repeat until `pipeline_done` or `pipeline_error`

## Pipeline Status via Tasks

Use **TaskCreate** and **TaskUpdate** to maintain a live floating task list — one task per stage. This gives the user an updating panel rather than repeated status blocks.

### At `pipeline_start`

Create one task per stage declared in the pipeline (inspect the `.line` file or use `thruline inspect` to get the stage list):

```
TaskCreate: "[pipeline-name] stage-name"   status: pending
```

Name each task `[<pipeline>] <stage>  (<runner> / <model>)` if the runner info is available from the inspect output. Use `pending` status for all.

### At `stage_invoke`

Update the task for the invoking stage:

```
TaskUpdate: status → in_progress
            description → "running — <artifact-name>: …"  (if prior outputs exist)
```

Then spawn the agent. If the `stage_invoke` event contains a `parallel` field, tell the agent it should use subagents:

- `"parallel": null` or `"parallel": {}` — append to the agent's prompt: *"You may spawn subagents to complete this task in parallel. Synthesize their results before returning your output."*
- `"parallel": {"limit": N}` — append: *"You may spawn up to N subagents to complete this task in parallel. Synthesize their results before returning your output."*

The agent's runner spec and declared outputs are unchanged — the parallel hint only affects how the agent chooses to work internally.

### After agent completes and resume succeeds

Update the completed stage task:

```
TaskUpdate: status → completed
            description → "<artifact>: <value>, …"
```

If a `route_taken` event names the next stage, update that task to `in_progress`.

### At `pipeline_done`

Update any remaining `in_progress` task to `completed`. Write a single summary line to the conversation:

```
✓ pipeline_done  [run: <run-id>]  — <N> stages complete
```

### At `pipeline_error`

Update the failing stage task to show the error. Write the error to the conversation and halt.

## Agent Output Protocol

When invoking a stage's agent, include these instructions in the prompt:

> Respond with a JSON object mapping output artifact names to their values.
> For file artifacts: write the file to disk and return the path as `file:///absolute/path`.
> For ref artifacts: return the value as a plain string.
>
> Example:
> ```json
> {
>   "verdict": "approved",
>   "spec": "file:///tmp/spec.md"
> }
> ```

## Event Handling Summary

| Event | Task action | Conversation output |
|-------|-------------|---------------------|
| `pipeline_start` | Create one task per stage (pending) | None |
| `stage_invoke` | Set stage task → in_progress | None (task panel shows it) |
| `stage_complete` | Set stage task → completed with artifact values | None |
| `route_taken` | — | None |
| `parallel_start` | Set stage task description to "fan-out: N slots" | None |
| `parallel_done` | Set stage task → completed | None |
| `pipeline_done` | Set any open tasks → completed | `✓ pipeline_done [run: <id>]` |
| `pipeline_error` | Set failing task description to error | Error message |

## Invoking the Stage Agent

When you receive a `stage_invoke` event, the payload contains:
- `runner.model` — the model to use
- `runner.system` — the system prompt (already resolved, not a file path)
- `runner.tools` — list of tools to grant
- `artifacts` — input artifact values as JSON
- `prompt` — optional task-level prompt

Construct the Agent call with the runner's system prompt as context. Pass all input artifacts and the task prompt to the agent. Tell the agent to respond with the JSON output protocol above.

After the agent completes, extract its JSON output and call:
```bash
thruline resume <run-id> --stage <stage-name> \
  --artifact verdict=approved \
  --artifact spec=file:///path/to/spec.md
```

## Error Handling

- **Binary not found**: Tell the user to install with `cargo install --path .` from the thruline repo, or add it to PATH
- **Stage agent failure**: Resume with an error artifact: `thruline resume <run-id> --stage <name> --artifact error=<message>`. Then halt and report the error.
- **Validation failure**: `thruline run` will exit non-zero with validation errors — display them and halt.

## Prerequisites

- `thruline` binary must be in PATH
- For the companion skill (stdio driver): no API key needed — Claude Code drives agents
- For standalone runs (api driver): `ANTHROPIC_API_KEY` must be set in the environment
