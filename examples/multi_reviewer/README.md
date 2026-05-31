# Multi-Perspective Code Review

Runs three parallel reviewers (security, performance, quality) via run blocks, then synthesizes into one report.

**Stages:** `review` (3 parallel run blocks) → `synthesize`

Demonstrates: `run` blocks (parallel invocations within a stage), value constraints.

## Run standalone

```bash
ANTHROPIC_API_KEY=sk-... thruline run review.line --driver api \
  --input code=file:///path/to/file.rs
```

## Run in harness mode

```bash
thruline run review.line --input code=file:///path/to/file.rs
# → emits parallel_start + 3x stage_invoke with "run" field
thruline resume <id> --stage review --run security_check \
  --artifact security_issues="None found." --artifact security_severity=low
thruline resume <id> --stage review --run performance_check \
  --artifact perf_issues="Consider caching." --artifact perf_severity=medium
thruline resume <id> --stage review --run quality_check \
  --artifact quality_issues="Functions too long." --artifact quality_severity=medium
# → all runs done → stage_invoke for synthesize
```
