---
name: thruline
description: Run a .line pipeline using the thruline binary as orchestrator. Drives agent invocations through this Claude Code harness and renders live pipeline status. Use when asked to run a thruline pipeline or .line file.
---

# Thruline Skill

Runs a `.line` pipeline file using the `thruline` binary as the orchestrator.
This skill bridges the binary's stdio event protocol with Claude Code's Agent tool.

## Usage

Invoke with a path to a `.line` file:
```
/thruline path/to/pipeline.line [--pipeline <name>]
```

## How It Works

1. Run `thruline run <file.line> --driver stdio` as a subprocess
2. Read NDJSON events from stdout line by line
3. On `stage_invoke` event — spawn the stage's agent using this harness's Agent tool, passing the runner spec and input artifacts
4. Call `thruline resume <run-id> --stage <name> --artifact key=value [...]` with the agent's output
5. Repeat until `pipeline_done` or `pipeline_error`
6. Render live pipeline status after each event

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

## Pipeline Status Display

After each event, output an updated status block. Use this format:

```
pipeline: <name>  [run: <run-id>]

  ● <stage>  (<runner-name> / <model>)   ✓ complete
  │  <artifact>: <value>, ...
  ⟳ <stage>  (<runner-name> / <model>)   running
  │
  ○ <stage>  (<runner-name> / <model>)   pending
  │
  ○ <stage>[*N]  (<runner-name> / <model>)   pending  (parallel, max N)
```

Legend:
- `●` = complete
- `⟳` = currently running
- `○` = pending

## Event Handling

| Event | Action |
|-------|--------|
| `pipeline_start` | Display pipeline name and run ID. Mark all stages pending. |
| `stage_invoke` | Use Agent tool to invoke the runner. Pass system prompt, model, tools, artifacts, and task prompt from the event payload. |
| `stage_complete` | Mark stage complete. Show output artifact values. |
| `route_taken` | Note which route was followed. |
| `parallel_start` | Note fan-out: N agents will be spawned for this stage. |
| `parallel_done` | All parallel slots complete. |
| `pipeline_done` | Display success. List output file paths. |
| `pipeline_error` | Display error. Halt. |

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
