# Journal claude-0013: pin bump 789d6c8 -> 894fa65 (WP1 + WP3)

Branch `claude/r2-bridge-gaps`. Plan: `docs/AGENT_CORE_PIN_BUMP_2_ANALYSIS_CLAUDE.md`. Decisions: ADR 0009.

## First RED evidence
- `tests/wire` (pin 894fa65, derived seed id): 14 failed (no wire dir, fallback sha, `seeded_release_id` missing).
- `tests/l3a/test_inflight_409.py`: TypeError `InvokeService() got an unexpected keyword argument 'sleep'`; after the retry
  loop the re-read of the parked receipt returned `manual_reconcile` instead of `unknown`, which exposed the reconciler
  gap and produced the `core_reservation_pending` rule.
- `tests/runtime/test_rate_limits_env.py`: 5 failed (`limits_from_env` / `app.state.pulso_limits` absent).
- Reconciler window tests were written together with the implementation (RED by construction: unknown `now=` kwarg).

## Results (PG16 `pulso-claude-u-pg`, pulso-dev, --cgroups=disabled, 127.0.0.1:47641, removed afterwards)
- `core-bridge/tests` against the 894fa65 venv: 561 passed, 5 skipped, 0 failed (sequential, ~9 min). `test_expand_contract`
  4 passed (OLD 789d6c8 venv + NEW 894fa65 checkout). No stuck tests.
- `gen-wire.ps1 -Check`: "wire check OK (no drift)"; `assert_compat()`: ok. MANIFEST digest `ed000b81...809b5` (ADR 0009).
- Wire diff vs 789d6c8: 10 changed files, 0 added, 0 removed, `openapi.json` identical.
- `platform-sim/tests` (out of scope, run only to size WP2): 82 failed, 8 errors, 118 passed.

## Notes
- `uv sync --locked` inside `gen-wire.ps1` strips `jsonschema`/`referencing` from the shared venv; reinstall
  `core-bridge/runtime-requirements.txt` into `%TEMP%\pulso-wire-venv-894fa65` before running tests afterwards.
- `references\agent-core-894fa65` does not exist (references are read-only for us); scripts default to that path, runs used a
  scratch clone at 894fa65 via `-Checkout` / `PULSO_CORE_CHECKOUT`.
- Not touched (Codex, WP5): `core-bridge/Dockerfile` `CORE_SHA` ARGs (still 789d6c8; `tests/runtime/test_image.py` does not
  compare them with `PIN_SHA`), workflows, compose, `e2e-core`, `contracts/agent_core/pin.json`.

## Remaining for WP2 (platform-sim)
Old id `rel-98130317a1003849` in `registry_mock/sim_common.py`, `tests/bridge_contract/test_bridge_contract.py`,
`tests/parity/test_mock_mutation_and_sim.py`; `registry_mock/{a2_app,app,real_app}.py` and `tests/parity/servers.py` name
789d6c8; re-record `fixtures/agent_core_wire/789d6c8/*` at 894fa65 with `record.py --target real`; mock fidelity:
`Interrupt.locked`, admin-only `interrupts` in `release_settings`, `max_input_chars` cap, REG-LOCKED,
`ApprovalReview.release_changes`, `Agent.input_schema` + AG-04.

## Remaining for WP4 (assets)
Regenerate `agent-core-assets/expected-state.json` (attention-demo `rel-98130317a1003849` -> `rel-da313458b550780a`,
ReleaseDetail with `locked`), refresh the byte-identical `registry-demo/releases/demo.yaml` copy, pin SHA in
`manifest.yaml` / `tests/conftest.py` / `tools/assetcheck.py`; optional `input_schema` decision.
