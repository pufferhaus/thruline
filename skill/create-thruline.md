---
name: create-thruline
description: Scaffold a new .line workflow file. Ask the user about their pipeline goals, then generate a complete, validated .line file with appropriate runners, stages, and routing. Use when asked to create a new thruline workflow or .line file.
---

# Create Thruline Skill

Scaffold a new `.line` workflow file by gathering requirements and generating a complete, validated file.

## Usage

```
/create-thruline [description of what you want to build]
```

## Process

### Step 1: Gather requirements

Ask the user (one question at a time if interactive):

1. **What is the goal?** What should the workflow accomplish?
2. **What are the inputs?** What does the user provide at the start? (files, text, etc.)
3. **What should it produce?** What artifacts matter at the end?
4. **Are there branches?** Does any step produce a pass/fail decision, or route to different paths?
5. **What model should run it?** (default: claude-sonnet-4-6)

If the user gave a description when invoking, extract answers from it before asking.

### Step 2: Design the stages

Map the workflow to stages:
- Each stage = one discrete agent task
- Identify what each stage consumes and produces
- Find decision points → predicate routes
- Find loops (e.g. retry until approved) → back edges
- Find parallel work → fan-out hints `[*N]`

### Step 3: Write the .line file

Generate a complete `.line` file following this structure:

```
config {
  model: <chosen-model>
}

// Runners — one per distinct agent role
runner <name> {
  system: "<system prompt describing the agent's role>"
  temperature: <0.1–0.3 for structured output, 0.5–0.8 for creative>
}

// Stages — one per discrete task
stage <name> {
  in:  <artifact> as <kind>
       <artifact>? as <kind>   // optional inputs with ?
  out: <artifact> as value     // use value for strings/decisions
       <artifact> as path      // use path for files
  runner: <runner-name>
  prompt: "<specific task instruction>"
}

// Thruline — routing logic
thruline <workflow-name> {
  inputs {
    <name> as <kind>
    <name>? as <kind>
  }
  start: <first-stage>
  routes {
    <routes...>
  }
}
```

### Step 4: Validate

Run `thruline validate <file.line>` to confirm the file parses and validates. Fix any errors.

### Step 5: Show run command

Print the command to run it:

```bash
# Standalone (Anthropic API):
ANTHROPIC_API_KEY=sk-... thruline run <file.line> --driver api \
  --input <key>=<value>

# Harness mode (Claude Code drives agents):
/thruline <file.line>
```

---

## Language Quick Reference

### Artifact kinds
- `as value` — in-memory string (decisions, short text, JSON)
- `as path` — disk file path (large files, code, documents)

### Optional inputs
- `artifact? as value` — stage runs even if this input is absent

### Route forms
```
stage-name -> next-stage                    // unconditional
stage.artifact == "value" -> next-stage     // predicate
stage.artifact != "value" -> next-stage     // negative predicate
stage-name -> next-stage[*3]               // parallel hint (up to 3 subagents)
stage-name[*] -> next-stage                // after fan-out completes
```

### Artifact flow
Artifacts flow automatically by name — no explicit wiring needed. If `classify` produces `language as value`, any later stage can declare `in: language as value` and receive it.

To pin to a specific stage's output: `classify.language as value`  
To pin to the original pipeline input: `input.code as value`

### Common patterns

**Retry loop:**
```
routes {
  review.verdict == "rejected" -> draft    // loop back
  review.verdict == "approved" -> publish  // exit
}
```

**Multi-branch:**
```
routes {
  classify.category == "bug"     -> triage
  classify.category == "feature" -> backlog
  classify -> inbox              // fallthrough for anything else
}
```

**Fan-out (parallel work):**
```
routes {
  prepare -> analyze[*3]   // run analyze with subagent hint
  analyze[*] -> synthesize // after all parallel work done
}
```

---

## Tips for Good Stage Design

- **One decision per stage.** A stage that produces a `verdict` should only produce a verdict, not also rewrite the document.
- **Small outputs for routing.** Route predicates only work on `value` artifacts — keep routing signals short strings (`"approved"`, `"rejected"`, `"bug"`, `"feature"`).
- **Prompts are instructions, not system prompts.** The `prompt:` field is a task instruction passed alongside the runner's system prompt. Put role/persona in `system:`, task details in `prompt:`.
- **Optional inputs for graceful starts.** If a stage might run before a prior stage has produced an artifact, mark it optional with `?`.
