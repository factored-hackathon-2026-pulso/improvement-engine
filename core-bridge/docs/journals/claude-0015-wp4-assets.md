# Journal claude-0015: WP4 agent-core-assets at 894fa65

Branch `claude/r2-bridge-gaps`. Plan: `docs/AGENT_CORE_PIN_BUMP_2_ANALYSIS_CLAUDE.md` (WP4).

## First RED
`pytest tests` against a clone at 894fa65: 4 failed, 18 passed (pin SHA, attention-demo fixture copy, expected-state, in-memory import).

## Changes
- Pin SHA 894fa65 in `manifest.yaml`, `tests/conftest.py`, `tools/assetcheck.py` (+ README); checkout lookup name `agent-core-894fa65`.
- `worlds/attention-demo/releases/demo.yaml` refreshed byte-for-byte from the upstream fixture (`locked: true`).
- `expected-state.json` + manifest digests regenerated with `assetcheck.py write-state` against the real Core.
- CORRECTION to the analysis: the real attention-demo release id is `rel-e26df0070f6be82f` (matches the WP1 gen_wire derivation),
  NOT `rel-da313458b550780a` (the analysis value came from an intermediate probe). The four pulso-evolution ids are unchanged.
- `test_state_matches_expected_state...` assertion updated to the verified id.

## input_schema decision: NOT added
AG-04 does not hit the four task agents (`agentcore validate` clean; flows read `facts.binding.value.*`, never `slots.*`).
Declaring `input_schema` would change the four agents' entity hashes and release ids and make Core reject (slot_not_accepted /
slot_type_mismatch) any run input the bridge sends that is untyped or absent (evaluate_enabled, optional suite version), which
cannot be verified inside this package (needs core-bridge runtime + PG integration). Left as a later separate decision.

## Results
`assetcheck.py check` and `validate`: ok (both worlds + merged). `pytest tests`: 22 passed.
