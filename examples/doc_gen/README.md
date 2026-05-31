# Documentation Generator

Drafts documentation for a code file, reviews it, and revises up to 4 times until approved.

**Stages:** `draft` ↔ `review` (loop, [max:4]) → `finalize`

Demonstrates: retry loop with `[max:N]` cap, value constraints on verdict, optional inputs.

## Run

```bash
ANTHROPIC_API_KEY=sk-... thruline run doc.line --driver anthropic \
  --input code=file:///path/to/module.rs
```
