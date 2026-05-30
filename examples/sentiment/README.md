# Sentiment Analysis

Classifies text sentiment (positive / negative / neutral) and routes to a tailored response stage.

## Run standalone (Anthropic API)

    ANTHROPIC_API_KEY=sk-... thruline run pipeline.line --driver api --input text="I love Rust!"

## Run in harness mode (stdio driver)

    thruline run pipeline.line --input text="I love Rust!"
    # → emits stage_invoke; drive the agent, then:
    thruline resume <run-id> --stage classify \
      --artifact sentiment=positive \
      --artifact score=9 \
      --artifact notes="Enthusiastic and appreciative tone."
    # continues routing until pipeline_done
