# Journal claude-0001: L1 wire snapshot, registry mock, a2 harness, bridge mock

Contract revision: `pulso-two-teams-1`. Agent Core pin: `86a767474042a566a0dbd6ed23588959f27ebdb3` (contracts 1.3.0).
Package: L1 (plan 17.3.1). Directories: `core-bridge/wire/`, `core-bridge/scripts/{gen-wire,test}.ps1`,
`platform-sim/{registry_mock,bridge_mock,fixtures,tests}`. Commits: 17b8e6f (L1a), dae9296 (L1b), 319c021 (review),
26dd6bd (bridge_mock). Documentation pass at HEAD `4984d92`.

## Purpose
Give Pulso a byte-exact copy of the pinned Core wire (schemas, OpenAPI, events, golden hash vectors), a faithful HTTP
registry mock (CAP-52) and a bridge wire mock (CAP-53), and prove mock fidelity with one parity suite
(`registry-wire-contract`) that runs against the mock, an "a2" harness (the real `RegistryService` over an in-memory
store) and, when available, a real server.

## Flow
`gen-wire.ps1 [-Check]` -> `gen_wire.py` (scratch venv outside the repo) -> `core-bridge/wire/agent_core@86a7674/`
(193 schemas + 2 events + `openapi.json` byte-copied, 13 derived model schemas + 6 request bodies +
`registry_openapi.json` flagged `derived_by_pulso`, golden vectors, `MANIFEST.json`). Parity: cases in
`platform-sim/tests/parity/cases` (about 100 YAML cases; 102 reported) are recorded against a2 into
`platform-sim/fixtures/agent_core_wire/86a7674/*.json` and replayed against the mock.

## Input / output
In: pinned checkout (HEAD must equal the pin). Out: wire snapshot, fixtures, `parity_report.<target>.json` under
`platform-sim/tests/parity/.out/` (never claims `real_local` from mock or a2). Mock: 16 `/v1/registry` routes, EdDSA JWS
auth, 12 registry error codes with `application/problem+json`, limits and quotas with an injectable clock, `/_sim/*`
(info, reset, clock, eval, fault: status500, latency, drop_after_commit, disconnect). Bridge mock: 6 `/internal/v1`
routes plus `/_sim`, bodies validated against `bridge_mock/schemas/*.json`, label `runtime_profile=contract_mock`.

## Transactions and idempotency
Mock holds in-memory state; publish requires `Idempotency-Key` (1..255 chars), same key and same proposal replays the same
release, same key other proposal conflicts. Bridge mock reproduces single-flight by `(tenant, Idempotency-Key)` with
digest conflicts and the receipt state machine.

## Errors
Registry codes: `validation_failed`, `gate_failed`, `proposal_stale`, `candidate_changed`, `illegal_transition`,
`forbidden_role`, `step_up_required`, `integrity_error`, `not_found`, `loosening_not_accepted`, `idempotency_conflict`,
`quota_exceeded`. Bridge mock errors use the D.1 envelope (`bridge_busy` 429 retryable, `core_unavailable`,
`run_not_found`, cross-tenant invisibility, release pin errors).

## Permissions
Mock auth: EdDSA JWS principals (roles `constructor`, `aprobador`, admin; bots cannot approve/publish/promote). Bridge
mock: service JWT with per-route `purpose` and tenant claim equal to the body tenant (same rules as ADR 0005).

## Config
`PULSO_WIRE_DIR` (wire snapshot location), `REGISTRY_BASE_URL` (real target), `TARGET=mock|a2|real`.

## Observability
Parity report with per-case result and class (`mock_infidelity`, `wire_drift_detected`); `MANIFEST.json` carries the pin
and a manifest digest checked against `pin.json`.

## Commands (as defined; head `4984d92`)
- `pwsh core-bridge/scripts/gen-wire.ps1 -Check` (drift check)
- `pwsh core-bridge/scripts/test.ps1 -Suite wire -Target mock|a2|real_local`
- `pwsh core-bridge/scripts/ci.ps1 -Job contract-drift|mock-wire|a2-wire|real-wire|platform-sim`
Environment of the recorded runs: Windows 11, Python 3.12 via `uv --python 3.12`, the pinned venv
`%TEMP%\pulso-wire-venv-86a7674`, no Postgres needed for L1.

## RED / GREEN
First RED: parity against a missing mock failed (no server); then each of four single-field mutations of the mock (drop
`candidate_hash`, problem without `type`, publish without key, wrong create status) was detected as `mock_infidelity`.
GREEN as reported in BITACORA: parity green on mock and a2 (102 cases), bridge_mock 60 contract tests. Not re-run in
this documentation pass (the shared pinned venv at `4984d92` lacked `jsonschema`, so part of the suite did not collect;
see journal claude-0004).

## Trade-offs
FastAPI mock (ADR 0001) over a Rust stub: fast to keep faithful but a second implementation to keep in parity. Mock
candidate validation is a documented subset; a2 is the oracle.

## Gaps
- Mock candidate validation covers REG-KIND, REG-VERSION, REG-VERSION-TAKEN, REG-UNREFERENCED and limits only; node
  (200) and scenario (200) limits and M1 flow rules were not fully covered at the time of the first entry (later review
  added 201-scenario REG-SCHEMA and node/scenario/idempotency-key cases).
- Cases needing a programmable evaluator are skipped on the real target; no real-PG recording done (`record-wire` is
  manual).
- Not simulated in bridge_mock: real Flow execution, real registry validation, real broker.
