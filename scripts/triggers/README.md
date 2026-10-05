# Trigger service prototype (TR1)

`agentcore_poller.py` is a pull-only trigger service: it polls agent-core and emits engine automation requests.
Python standard library only; contacts loopback hosts only; tokens never printed, logged or stored in state.

## Sources (agent-core `/v1/export`, role `exporter`)

| Endpoint | Cursor | Becomes |
|---|---|---|
| `GET /v1/export/runs?after&limit` | `cursor` of the last run (state `runs_cursor`) | closed runs -> `run.closed` |
| `GET /v1/export/runs/{id}/events?after&limit` | `seq` per run (state `run_event_cursors`) | supplies the `run_closed` event id |
| `GET /v1/export/registry-events?after&limit` | count of events read (state `registry_after`) | `published`, `promoted`, `revoked` -> `release.published|promoted|revoked` |

Pages are `{items, next_after}`. The cursor is saved only after the sink accepted the whole poll; `--overlap N`
re-reads the last N positions; duplicates are dropped by key. Registry events carry no id, so their identity is
`position:type:release_id`. `--only-origin auto_detect` keeps our proposals; `--only-agent` filters runs.

## Request: `pulso.trigger.v1`

```json
{"schema": "pulso.trigger.v1", "trigger_key": "sha256:...", "kind": "outcome|explicit|scheduled",
 "tenant": "...", "mission": "...", "source": "agent-core", "config_digest": "...",
 "event": {"type": "run.closed|release.*|run.now|schedule.tick", "ref": "...", "at": "...", "subject": {}},
 "requested_at": "..."}
```

`trigger_key = sha256([tenant, mission, source, config_digest, kind, ref])` (spec section 563: tenant, mission,
source and config digest are in every trigger). The key is also the `Idempotency-Key` header. `explicit` uses the
caller's key as `ref`; `scheduled` uses the slot `now // interval`.

## Engine side: gap and minimal additive change

On origin/main the debug-api `/internal/v1/automation/*` is a read model (`case-types`, thresholds `PUT config`) plus
SIMULATED `approve` / `publish-staging`; `pulso monitor` triggers through the source watermark, not over HTTP. There
is no endpoint that accepts a trigger request. Minimal additive change (owner L-CAPI, not made here):
`POST /internal/v1/automation/triggers`, bearer + CSRF like `proposal_action`, body `pulso.trigger.v1`,
idempotent on `Idempotency-Key` (replay returns 409 or the first response), audit event `automation_trigger_received`.
The endpoint now exists (TR2, served by `pulso run` with its real job store). `--engine-url` POSTs to it with the bearer and the `X-CSRF-Token` read from `GET /api/v1/auth/session` (kept in memory, refreshed once on a 403). `--out FILE` (JSONL, idempotent on key) remains for offline use.

## agent-core: no change needed

All three endpoints exist in the local stack (head `f91ac44`). Needs only the `exporter` role: the local stack has
no exporter token minted, so the live test used the local `admin` token (admin passes `require_exporter`). In
shared envs infra's staff issuer must mint `exporter`. Nit: registry events lack an `event_id`.

## Run and test

```
python -m unittest discover -s scripts/triggers -p "test_*.py"
python scripts/triggers/agentcore_poller.py poll --once --token-file .dev-stack/tokens.json --out triggers.jsonl
python scripts/triggers/agentcore_poller.py explicit KEY --out triggers.jsonl
python scripts/triggers/agentcore_poller.py scheduled --interval-secs 3600 --out triggers.jsonl
```

Evidence labels: unit tests use a fake core (simulated); `fixtures/export_recorded.json` is RECORDED from the local
stack; a live run against the local agent-core (claude/l3-local-stack, `/v1/export`) emitted 6 `run.closed` and a
silent replay. No `release.*` was observed live because no release was published locally (mapping tested on
simulated pages only).
