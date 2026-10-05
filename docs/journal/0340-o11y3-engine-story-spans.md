# 0340 O11Y3 engine story spans (UTC 2026-10-05T16:00Z, CLAUDE)

Lane O11Y3 (plan W1-3), branch `claude/o11y3-engine-spans` (o11y1 bridge + b3-wire merged, clean).

Delivered
- `scripts/o11y/engine_trace.py`: value-loop outcome + debug-api events (file, bundle or loopback API with the debug token from env/file) -> one OTLP trace per finding under the story trace id: root `pulso.story`, `stage.*` spans with outcome/error, `generation <role>` spans (model, usage, price-table USD, prompt/response when recorded), scores `gate_regression_proven`, `rubric_total`, `announce`, `outcome_class`. Local receiver by default; Langfuse only with `--allow-external`.
- `trace_id.py`: `traceparent_for(finding_key, run_id, stage, attempt)`, root/stage/generation span ids; `fixtures/traceparent_vectors.json` for the Rust side; derivation in `docs/dev/O11Y.md`.
- Recorded fixture `fixtures/engine_story_recorded.json` (shape from code and b3 live-pass aggregates; model calls, verdict, dossier of finding 0 are synthetic).

Gap (documented, contract `pulso.model_call/1` in O11Y.md): the engine records no prompts, responses, tokens or timings per model call; `/runs/{id}/model-calls` is an empty page. Stage times are inferred from the event window and flagged.

Verified: 47 offline tests; the three recorded stories (24 spans, 3 traces) accepted by the local receiver with a valid shape.
Not done: live engine run (no cargo or docker here); nothing sent to Langfuse Cloud; Rust `traceparent_for` and `model_call` recording are for the engine lane.
