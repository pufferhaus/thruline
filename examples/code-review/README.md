# Code Review

Reviews code, loops on rejection until approved, produces a final summary.

**Stages:** `review` → (rejected) `revise` → `review` → (approved) `summarize`

## Run standalone (Anthropic API)

    ANTHROPIC_API_KEY=sk-... thruline run review.line --driver api \
      --input code=file:///path/to/file.rs

## Run in harness mode (Claude Code)

    /thruline review.line --input code=file:///path/to/file.rs

## Run in harness mode (stdio driver)

    thruline run review.line --input code=file:///path/to/file.rs
    # → emits stage_invoke for "review"; drive the agent, then:
    thruline resume <run-id> --stage review \
      --artifact verdict=approved \
      --artifact feedback="Clean implementation, good error handling."
    # approved → routes to summarize
