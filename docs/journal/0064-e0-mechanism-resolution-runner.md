# P4 E0 mechanism resolution in the local runner

**Date:** 2026-10-03
**Owner:** CODEX
**Branch:** `feat/p4-e0-mechanism-resolution-runner`
**Stack:** P4 proposal activity `a62dc859e441cb3294d383256fe361beb57d00c8`,
then P3 resolver `469fc0f4b105fe6860bb3e13c23bd5799a1ec887` (cherry-picked as
`31f06eb`).

## Behavior

For a qualifying recurring-query candidate in an E0 local run, the runner
constructs the P3 candidate-bound evidence packet and resolves it against an
empty immutable catalog fixture. The result envelope is
`e0_mechanism_resolution`:

- `evidence_origin=e0_local_run` identifies the source of the packet.
- `catalog_origin=team_generated_empty_local_catalog_fixture` and
  `catalog_durability=ephemeral` describe only the empty catalog fixture.
- `evidence_packet` and `resolution` preserve the P3 serialized contracts.

The fixed UUIDv7 catalog reference has revision 1 and the canonical digest of
empty mapping content. Because the catalog has no route mappings, the only
resolution is `unlinked/no_exact_supported_flow_mapping`. The result does not
claim that a Core Flow exists, that a route is executable, or that any business
outcome improved. No qualifying candidate produces no packet and no mechanism
event. OriginalBank has no mechanism output.

When present, exactly one `e0_mechanism_resolution` RunEvent follows the
optional holdout and `proposal_assembly` events. It contains the catalog-only
origin/durability labels, aggregate counts, reason code, status and cutoff;
the event object is identical in `result.json` and `events.ndjson`. Existing
staging persistence and cleanup behavior is retained.

## TDD and verification

- Initial RED on the public CLI recurrence fixture: the expected
  `e0_mechanism_resolution` field was absent. The test reached the behavior
  assertion after successful compilation and execution.
- First GREEN after runner composition: focused recurrence CLI test passed.
- Independent adversarial review identified that the initial envelope used a
  team-generated origin for the entire resolution, obscuring that its evidence
  came from the E0 run. The regression was tightened first and RED observed
  (`evidence_origin` was null); the envelope now separates evidence origin
  from catalog origin/durability, and event detail labels only the catalog as
  team-generated.
- Final focused runner tests:
  `CARGO_TARGET_DIR=.target-p4-mechanism-resolution CARGO_BUILD_JOBS=2 cargo
  +1.98.1 test --locked --offline -p improvement-engine-runner --tests` —
  8 unit + 14 CLI E2E + 2 proposal-output tests passed (24/24).
- `cargo +1.98.1 fmt --all -- --check` — passed.
- `CARGO_TARGET_DIR=.target-p4-mechanism-resolution CARGO_BUILD_JOBS=2 cargo
  +1.98.1 clippy --locked --offline -p improvement-engine-runner --all-targets
  -- -D warnings` — passed (exit 0).

## Limitations

The catalog is an ephemeral local fixture, not a Core registry snapshot and
not durable state. This slice accepts no catalog file and cannot produce a
mapped result. The separately tested P3 mapped case is a team-generated
contract fixture only. The runner output is local descriptive evidence, not
Agent Core `EngineEvent`, publication, execution, evaluation, or business
lift. Full workspace local CI and actual-data E0/OriginalBank smokes have not
been rerun for this follow-on.

Owned files: `crates/runner/src/main.rs`,
`crates/runner/tests/cli_e2e.rs`,
`crates/runner/tests/e0_proposal_output.rs`,
`docs/data/e0-proposal-assembly.md`, `docs/local-e0-e2e-runner.md`,
`docs/IMPLEMENTATION_STATUS.md`, and this journal entry.
