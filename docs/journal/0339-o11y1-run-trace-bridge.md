# 0339 O11Y1 run-trace bridge (UTC 2026-10-05T03:00Z, CLAUDE)

Lane O11Y1, branch `claude/o11y1-run-trace-bridge`.

Delivered
- `scripts/o11y/runtrace_bridge.py`: agent-core `/v1/export/*` pull with persisted cursor (reuses the poller's fetch, loopback guard and redaction) -> one OTLP/HTTP JSON trace per closed run (root agent span, one span per audit event, reason fields as `pulso.reason.*`, `gen_ai.*` and `langfuse.*` attributes, session = run id, price-table cost), registry events as small traces, per-trace boolean scores, idempotent model registration. Local receiver `scripts/o11y/otlp_receiver.py` validates the OTLP shape.
- Policy: local loopback target by default; Langfuse target only with `--allow-external` and https; credentials from environment only; content off unless `PULSO_O11Y_CAPTURE_CONTENT=1`, with scrub.
- `docs/dev/O11Y.md`: how to point llm-gateway and agent-core at the same backend, and the traceparent linkage.

Verified
- 19 offline unit tests (synthetic events covering every mapped type, privacy, policy, cursor holdback, score queue, bridge to receiver).
- Live: private agent-core on :8012, two real `pulso-builder` runs, bridge -> local receiver: 2 run traces (44 spans) and 5 registry traces accepted with a valid shape.

Not done
- The send to Langfuse Cloud and the read-back via the public API were not executed: the tool permission layer blocked the external call, so it needs the user's own go-ahead.
- `agent_step` events carry no tokens, so ReAct-node cost is 0 on run traces until the gateway spans are joined by traceparent.
