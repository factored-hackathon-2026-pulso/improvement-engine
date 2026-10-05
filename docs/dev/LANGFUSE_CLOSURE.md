# Langfuse closure: one command (LFC)

`scripts/o11y/langfuse_closure.ps1` brings a LOCAL stack up, generates real traffic, sends it to Langfuse Cloud and VERIFIES it by
reading Langfuse back. Windows PowerShell 5.1 and pwsh; it changes to the repository root itself, so the current directory does not matter.

## What you run

```
D:\.codex\factored\worktrees\improvement-engine-claude-lfc\scripts\o11y\langfuse_closure.ps1 -All
```

`-All` = `-Models -Up -Traffic -Verify -Down` (teardown also runs when a step failed; `-KeepUp` leaves the stack up). Prerequisites: Podman machine
`pulso-dev` running, `uv`, Python 3.11+, `D:\.codex\factored\langfuse.env` (names `LANGFUSE_SECRET_KEY`, `LANGFUSE_PUBLIC_KEY`, `LANGFUSE_BASE_URL`),
`agent-core.env` and `llm-gateway.env` next to it, and a built engine (`pulso.exe`, found under `D:\cargo-targets\claude-lfc\debug`).
Expect 10 to 20 minutes the first time (gateway image build, `uv sync`), then about 6 minutes per run. Model cost is a few cents (synthetic input).

| Step | What happens |
|---|---|
| `-Models` | checks the env file by key NAMES, pings `/api/public/health` (prints only the status), registers `xiaomi/mimo-v2.6-flash` 1.4e-7/2.8e-7, `xiaomi/mimo-v2.6-pro` 4.35e-7/8.7e-7, `z-ai/glm-5.3-flash` 1.5e-7/5e-7 USD per token (existing ones are skipped) |
| `-Up` | loopback forwarder on `127.0.0.1:4328` (holds the key, in its own process), stack `pulso-lfc` (Postgres 55510, gateway 8210 built from the PR 4 checkout with `LLM_GATEWAY_TRACE_CONTENT=1`, agent-core 8211 from the LOCAL scratch branch `scratch/lfc-agentcore` = origin/main + PR 48, never pushed, with `AGENTCORE_TRACE_CONTENT=1` and `AGENTCORE_TRACE_LANGFUSE=1`); both export OTLP/protobuf to the forwarder |
| `-Traffic` | 2 value-loop stories (SYNTHETIC planted cells, Builder `xiaomi/mimo-v2.6-pro`; one trace id per story shared by engine stage spans and generations, gateway spans and agent-core spans through the engine's `traceparent`), 2 plain agent runs (`disputas` es, `consultas` pt, `POST /v1/runs` with a `traceparent`), the run-trace bridge, the scores `gate_regression_proven`, `rubric_total`, `announce`, `outcome_class` |
| `-Verify` | reads `GET /api/public/traces`, `/observations`, `/scores` back (waits up to `-WaitSecs 240` for ingestion) and prints COUNTS only |
| `-Down` | stops the forwarder and the `pulso-lfc` containers and the scratch agent-core process; nothing else is touched |

Re-run a part: `-Models`, `-Up`, `-Traffic`, `-Verify`, `-Down` (any combination, always in that order). `-Mock` sends to a local MOCK Langfuse
(`scripts/o11y/mock_langfuse.py`, loopback) instead of the cloud: a complete dry run.

## What you should see (end of a good run)

```
traces: 4 (expected 4)
observations: ...  spans: ...  generations: ...
generations with input+output content: N          (N > 0)
observations by source: engine=.. gateway=.. agent-core=..    (all three > 0)
story traces with all three sources under one trace id: 2 of 2
models resolved with cost: 1+ (xiaomi/mimo-v2.6-flash, ...)
scores: 6+ (announce, gate_regression_proven, outcome_class, rubric_total)
VERIFY OK
```

`VERIFY FAILED` lists `PROBLEM:` lines, each naming what is missing and the likely cause (see below). Exit code 0 only when all expectations hold.

## How the credentials are handled

The Langfuse key is read ONLY by python children through `--env-file` (forwarder, `langfuse_verify.py`); it is never in this script's environment, never on a
command line, never printed. Every child line is masked with the values, their long fragments and the Basic header value. `langfuse_closure.Tests.ps1` has
canary tests (a child dumping its environment, needles, the stack environment, a bad command line from another directory); `test_langfuse_closure.py` has a
canary over the python CLI output. Engine content goes through the same secret scrub as before. Gateway and agent-core spans are protobuf and pass the
forwarder byte-exact (they are NOT maskable there: the emitters' own rules apply, see O11Y.md).

Container to host: Podman containers cannot reach a Windows loopback listener by default, so the gateway container runs with `--network host` and
`LISTEN_ADDR=127.0.0.1:8210` (`PULSO_GW_HOST_NETWORK=1` in `stack.py`; works with the mirrored WSL networking of this machine; measured both directions).

## Troubleshooting

| Symptom | Meaning / fix |
|---|---|
| `agent-core unreachable: URLError` | the stack is not up (torn down, or `-Up` never ran). Run `-Up` or `-All` |
| `langfuse env file: missing or empty key name(s)` | the file lacks one of the 3 names (values are never shown) |
| ping `HTTP 404` / `unreachable` | wrong region host in `LANGFUSE_BASE_URL` (US is `https://us.cloud.langfuse.com`) or no network |
| models step `HTTP 401` | the keys belong to another project / are wrong |
| `no GATEWAY spans` | Langfuse rejected `application/x-protobuf` from the forwarder (`.dev-stack\lfc\forwarder.log` shows `traces dropped: upstream HTTP 4xx`), or the gateway container has no `OTEL_EXPORTER_OTLP_ENDPOINT`. If Langfuse takes only JSON, the gateway needs a JSON exporter or a converting collector |
| `no AGENT-CORE spans` | same protobuf path; or agent-core is not the PR 48 build (`-AgentCoreDir`) |
| `only N of M story traces carry all three sources` | traceparent not propagated for that call, or an exporter had not flushed (rerun `-Verify`) |
| no generation has input AND output | `LLM_GATEWAY_TRACE_CONTENT` / `AGENTCORE_TRACE_CONTENT` not applied, or engine content capture off |
| no model resolved with cost | `-Models` not run for this project, or a model name outside the three patterns |
| `429` while reading back | Hobby limit (30 API requests per minute); the client throttles and retries; rerun `-Verify` |
| port already in use | another process owns 55510, 8210, 8211, 4210 or 4328 |
| engine / loop failures | see `docs/dev/DEMO_LOOP.md`; logs are in `.dev-stack\lfc\` (`closure-*.log`, `forwarder.log`, `engine-*.log`) |

## Files

`scripts/o11y/`: `langfuse_closure.ps1` (+ `.lib.ps1`, `.Tests.ps1`), `langfuse_verify.py` (health, models, read-back verification), `agent_runs.py` (plain runs),
`mock_langfuse.py` and `otlp_decode.py` (mock and protobuf decoder), `otlp_forwarder.py` (now also `GET /v1/stats`: pending/forwarded/failed counts),
`engine_trace.py` (the gate score now also comes from the record's own `evaluation`). `scripts/demo-loop/run.ps1` gained `-StackPrefix -PgPort -GwPort
-CorePort -EnginePort -StateName -FreshGateway` and keeps an engine snapshot (debug events and model calls) next to each loop result.
Tests: `python -m unittest discover -s scripts/o11y/tests -p "test_*.py"` and Pester on `langfuse_closure.Tests.ps1`.

## Verified offline and live-without-sending

Against the mock: auth, `x-langfuse-ingestion-version: 4`, JSON and protobuf, idempotent models, the read-back logic with fixtures that contain and omit each of the
three sources, a Langfuse that rejects protobuf, 429 retries. Live local stack (real gateway container, real agent-core, real engine, real model calls) with the
forwarder pointed at the mock: see journal 0660 for what the captured payloads showed (the three sources under one trace id).

## NOT verified until the real send

- that Langfuse Cloud accepts `application/x-protobuf` OTLP from the forwarder (documented as supported; never exercised against the cloud);
- the exact JSON of the cloud's `/api/public/traces`, `/observations`, `/scores` (the verifier follows the documented fields `type`, `input`, `output`, `model`,
  `calculatedTotalCost`; names of fields may differ slightly, in which case a count would read 0 and the diagnosis would say so);
- that Langfuse resolves the three models with cost from tokens (the mock mimics it) and that gateway-reported cost is not double counted with the engine generation;
- that observation names classify the source in the cloud exactly as in the mock (`pulso.story`/`stage.*`/`generation *` engine, `chat *` gateway,
  `agentcore.*`/`invoke_agent`/`execute_tool` agent-core; `service.name` is a fallback when exposed in metadata);
- ingestion delay and rate limits of the real project (the verifier waits up to 240 s and retries 429);
- the plain agent runs need agent-core's test issuer (local demo auth) and the e2e demo tools; if `disputas`/`consultas` do not reach an outcome the trace still exists.
