# platform-sim

Doubles and contract tests for the two wires Pulso does not own at runtime. Everything here is a **double**: it must be
reported in `doubles[]` of any evidence and must never be claimed as the real Core, control-api or lab-broker.
Contract revision: `pulso-two-teams-1`. Agent Core pin `789d6c89b2fca90fc10e2abf157da51dc81c5d51` (contracts 1.3.0).

| Piece | Path | What it is |
|---|---|---|
| Registry mock (CAP-52) | `registry_mock/` | FastAPI HTTP process, in-memory state, 16 `/v1/registry` routes, EdDSA JWS, 12 error codes, limits, quotas with an injectable clock, `/_sim/*` (info, reset, clock, eval, fault). ADR `core-bridge/docs/adr/0001`. |
| a2 harness | `registry_mock/a2_app.py` | the REAL `RegistryService` over `InMemoryRegistryStore`, sim staff verifier, scripted `EvalPort`; no `testing` import. The parity oracle. |
| Bridge mock (CAP-53) | `bridge_mock/` | the `/internal/v1` routes of `pulso-core-runtime` (invoke, read, aliases, dry-run, version, credentials) with `runtime_profile=contract_mock`; bodies validated against `bridge_mock/schemas/*.json`. |
| Ingest fixture | `ingest_fixture/` | server semantics of `pulso-observations-2` for the exporter: unknown fields rejected, server-side JCS digest, `Idempotency-Key == batch_digest`, `fast_poll` CAS on `expected_cursor_revision`, rescan never advances a checkpoint, dedup identity, 1 MiB artifacts. Not Codex's control-api. |
| Fixtures | `fixtures/agent_core_wire/789d6c8/*.json` | wire cases recorded against a2 (LF line endings, CRLF-proof digest). |
| Tests | `tests/parity`, `tests/bridge_contract` | `registry-wire-contract` (about 100 YAML cases, 102 reported) and 60 bridge contract tests against a real HTTP process. |

Fault injection exists only under `/_sim/*` and is off by default. Evaluation in the registry mock is
`contract_fixture` (programmable verdicts: pass, fail, failed_infra, timeout) and never simulates improvement.
`local/core/doubles/run_doubles.py` serves all three apps in one container for the standalone stack and adds
`GET /_sim/info` on the ingest app so evidence code can detect a double.

## Run

    python -m registry_mock.main --port 8601        # from platform-sim/ with PYTHONPATH=.
    python -m bridge_mock.main --port 8611

## Test

From `core-bridge/` with the pinned venv (see `core-bridge/README.md`):

    pwsh scripts/test.ps1 -Suite wire -Target mock    # or a2, real_local (needs REGISTRY_BASE_URL)
    python -m pytest -c pyproject.toml ../platform-sim/tests -p no:cacheprovider

Parity reports are written to `tests/parity/.out/parity_report.<target>.json`; `real_local` is never reported from a mock or
a2 run. A mismatch is `mock_infidelity` (mock) or `wire_drift_detected` (a2/real).

## Known gaps

Mock candidate validation is a subset of the real rules (REG-KIND, REG-VERSION, REG-VERSION-TAKEN, REG-UNREFERENCED,
limits, 201-scenario REG-SCHEMA); other rules are covered only by a2. Cases needing a programmable evaluator are skipped on
the real target. No real-PG recording was done (`record` is manual). The bridge mock does not execute Flows, validate
registry content beyond limits/kind, or simulate the real broker. Details: `core-bridge/docs/journal/claude-0001-l1-wire-mock.md`.
