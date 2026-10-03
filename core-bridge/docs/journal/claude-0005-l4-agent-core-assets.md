# Journal claude-0005: L4 agent-core-assets

Contract revision: `pulso-two-teams-1`. Pin `86a767474042a566a0dbd6ed23588959f27ebdb3` (contracts 1.3.0). Package: L4 (plan
17.3.4). Directory: `agent-core-assets/`. Commits: e33bfd8, 623f5fc (complete protected writer flow), 60042a9, 669cfb8,
447772b (Flows read inputs from `bind_context` facts). Documentation pass at HEAD `4984d92`.

## Purpose
Versioned Core assets for two worlds: `attention-demo` (pinned `registry-demo`: agent `atencion`, flow `disputa-cargo`,
suite `disputas-suite` with 3 scenarios, calibration `cal-demo`) and `pulso-evolution` (four task stages
`pulso-scout`, `pulso-verifier`, `pulso-builder-design`, `pulso-writer`, the Jev model `jev-evolution-route`, `pulso/*`
tools, generated `registry/*` ToolDefs from `BUILDER_TOOL_DEFS`, and a smoke suite `pulso-smoke`), plus the validator.

## Flow
`tools/assetcheck.py {check|validate|write-state}`: offline tier (PyYAML only: layout, manifest and
`expected-state.json` drift, stage invariants including "no precomputed finding", merge conflicts, layer mappings,
secrets) and an agent-core tier (uses the pinned checkout: `agentcore validate` per world and merged, state computed equals
`expected-state.json`, in-memory import where re-import is `illegal_transition`, a bot is `forbidden_role` and a session
admin is `step_up_required`).

## Input / output
In: `worlds/*` YAML, `manifest.yaml`, `layer_mappings/atencion@1.0.0.yaml`. Out: `expected-state.json` (generated, changed in
the same change as the asset). `manifest.yaml` carries the pin, per-world `files_digest`, `capability_catalog_digest`,
`expected_state_digest`, and the release ids: `atencion` `rel-98130317a1003849`, `pulso-scout` `rel-1cddd55d1fe8f19f`,
`pulso-verifier` `rel-db71ae5ed04c6131`, `pulso-builder-design` `rel-5f152bbf73ea31b4`, `pulso-writer`
`rel-bf3f06148962dabb`. The runtime seed (`local/core/init/seed_assets.py`) must reproduce these ids.

## Transactions and idempotency
Import is idempotent by content hash; the seed writes nothing on a second start (marker digest, see journal claude-0008).

## Errors
Violation codes from `assetcheck.Violation` (for example drift, forbidden precomputed-finding keys such as
`findings|winning*|precomputed*|golden*|known_*`, forbidden schema keys `const|default|examples`, waiting nodes
`collect|confirm|transfer|await_approval` in a task stage, secret-looking keys/values).

## Permissions
Task stages are `mode: task`, `invocable_by: [builder]`; the writer is the only one with `registry/*` tools (ADR 0003).

## Config
`AGENT_CORE_CHECKOUT` (optional pin checkout), `pytest.ini`; `ci/validate.sh` runs `check` then `validate`
(`uv run --python 3.12 --with pyyaml`).

## Observability
The validator prints one `code path: message` per violation and exits non-zero.

## Commands (head `4984d92`)
- `uv run --python 3.12 --with pyyaml python agent-core-assets/tools/assetcheck.py check|validate`
- `pwsh core-bridge/scripts/ci.ps1 -Job agent-core-assets`
- `pytest -c agent-core-assets/pytest.ini agent-core-assets/tests`
Environment: Windows 11, Python 3.12 via uv. Doubles in tests: in-memory registry store, `FakeEvaluator` (no Postgres, no LLM).

## RED / GREEN
First RED (`tests/test_policy_first_red.py`): a precomputed finding or a manifest/expected-state drift must fail
validation (offline tier). GREEN as reported in BITACORA: 16 tests pass in the first slice. Not re-run here.

## Trade-offs
Task-stage Flows read inputs from `facts.binding.value.*` (ADR 0004) instead of `slots.*`, coupling every Flow to the first
`pulso/bind_context` node. The stage catalogue lives in code (`stages/catalog.py`, L3) and the Flows here must match it (CI
drift check).

## Gaps
- New attention scenarios have no provider scripts (harness side).
- CODEOWNERS is outside this directory.
- `agent-core-assets` is not copied into the runtime image; seeding uses `local/core/init/seed_assets.py`.
- The first-slice entry reported that the writer flow omitted the proposal-exists branch and evaluate; commit 623f5fc
  ("complete protected writer flow") addresses this; not re-verified end to end in this pass.
