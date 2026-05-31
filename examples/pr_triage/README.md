# PR Triage

Classifies a PR description and routes to category-specific acknowledgment.

**Stages:** `classify` → (one of) `ack_bug` / `ack_feature` / `ack_refactor` / `ack_other`

## Run

```bash
ANTHROPIC_API_KEY=sk-... thruline run triage.line --driver anthropic \
  --input description="Add retry logic to the payment processor on timeout"
```
