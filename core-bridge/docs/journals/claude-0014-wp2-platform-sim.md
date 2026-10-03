# Journal claude-0014: WP2 platform-sim against agent-core 894fa65

Branch `claude/r2-bridge-gaps`. Plan: `docs/AGENT_CORE_PIN_BUMP_2_ANALYSIS_CLAUDE.md` section 5.4 / 7 (WP2). Paths touched: `platform-sim/**` only.

## What changed
- Pin constants: `registry_mock/sim_common.py::PIN_SHA` -> `894fa65575d83420523f33ec1c6919b8965f7ebe`; seeded release id
  `rel-98130317a1003849` -> `rel-e26df0070f6be82f` (verified against `core-bridge/wire/agent_core@894fa65/golden/hash_vectors.json`,
  `release_id` and `release_detail.release_id`; the seed interrupt `fraude` is now `locked: true`). Replaced in `sim_common.py`,
  `tests/bridge_contract/test_bridge_contract.py`, `tests/parity/test_mock_mutation_and_sim.py`. The 789d6c8 mentions in
  `registry_mock/{a2_app,app,real_app}.py`, `tests/parity/servers.py` and the README now name 894fa65 (default checkout / venv paths
  follow the `agent-core-894fa65` convention used by `gen-wire.ps1`).
- Fixtures: `fixtures/agent_core_wire/789d6c8` moved (git mv) to `894fa65` and fully re-recorded: a2, `real` (PG16, real
  `PgRegistryStore`) and `real_pg_scripted`. The old dir was removed: no test needs a dual pin (`FIXTURES` derives from `PIN_SHA`).
- Mock fidelity (all with parity cases recorded from a2/real first; new file `tests/parity/cases/release_settings.yaml`, 19 cases):
  `Interrupt.locked` in `ReleaseDetail`; `release_settings` draft kind (N-07) in the mock; admin-only `interrupts` (`forbidden_role`);
  `max_input_chars` ceiling 100000 (REG-SCHEMA at validate/freeze); REG-LOCKED (remove / lower priority / change action / unlock);
  `ApprovalReview.release_changes` (before/after vs base); `Agent.input_schema` (REG-SCHEMA when mode is not `task`) and a subset of
  AG-04 (drafted flow reads `slots.X` that no `collect` node, `input_schema` or `accepts` writes).
- Runner (`tests/parity/runner.py`): helpers `$settings`, `$agent_input_schema`, `$golden_unwritable_slot`; the normaliser now records
  `interrupts` as `id:priority:locked=...` and `release_changes` (the `normalise` change re-baselines every fixture, hence the full re-record).

## First RED evidence
`TARGET=mock pytest tests/parity/test_registry_wire_contract.py` after recording the new cases from a2 and before touching
`registry_mock/app.py`: 25 failed, 123 passed (20 new cases plus `evaluate-pass`, `evaluate-failed-infra-keeps-candidate`,
`approve-ok`, `publish-ok` whose review gained `release_changes`, plus the real-recording consistency tests while the real recordings
were still being written). After the mock change: the 20 mock-comparison failures went to 0 on the first run.

## Known gaps
- AG-04 and `input_schema` are a documented subset: AG-04 is checked only for flows drafted in the proposal, against the drafted agent
  or the base agent; `slots.X` detection is a prefix match on any string in the flow (Core uses its typed read sites). A valid `task`
  agent with `input_schema` is not in the parity cases (the seeded flow is conversational: AG-01 fires first); the slot types
  (`string|integer|decimal|date|boolean`) are not validated by the mock.
- The mock does not retarget a `start_flow` interrupt to the candidate's flow version, and `language_detection` /
  `injection_ruleset` settings are resolved against the candidate refs only when the entity is part of the release.
- `bridge-contract/` was not regenerated (later step).
