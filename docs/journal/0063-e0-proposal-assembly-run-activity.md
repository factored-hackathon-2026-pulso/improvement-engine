# E0 proposal-assembly run activity

**Date:** 2026-10-03
**Owner:** CODEX
**Base:** validated local P2 commit `5aa8e58f2098c91f45913011add72af26612c7fe`

## Behavior

The local runner persists exactly one aggregate-only `RunEvent` with stage
`proposal_assembly` for E0 runs. It is appended after the optional
`e0_recurrence_holdout` event and is written as the identical serialized
object in `result.json` and `events.ndjson`. Its status mirrors the bounded
assembly status; detail contains candidate/disposition counts only, and its
cutoff is copied from the run. OriginalBank runs do not emit this event.

The event is internal local-run observability. It is not an Agent Core
`EngineEvent`, customer-attention event, execution receipt, proposal
publication, evaluation result, or business-lift claim. No IDs, tenant values,
digests, candidate references, or source-row data are included in its detail.
The existing `LocalRunResult` schema remains unchanged.

## TDD and verification

- RED: on the validated P2 base, the public CLI E0 test compiled and failed
  because it observed zero `proposal_assembly` events instead of exactly one;
  the OriginalBank negative test passed.
- GREEN implementation: `persist_result` constructs the event only for E0,
  validates that pre-existing events and optional holdout have contiguous
  one-based sequence values, and uses checked increment so gaps/overflow fail
  closed. Persistence errors retain the existing staging-directory cleanup.
- A sequence unit test covers valid ordering, gaps, and checked capacity.
  Persistence tests cover the unsupported/empty E0 assembly status and confirm
  the same event appears in both outputs. CLI tests cover candidate-ready E0,
  full result/NDJSON event-vector parity, holdout-before-proposal order, and
  absence of the event for OriginalBank.
- `cargo +1.98.1 fmt --all`, `cargo +1.98.1 fmt --all -- --check`, and
  `git diff --check` passed.
- `CARGO_TARGET_DIR=.target-p4-proposal-activity CARGO_BUILD_JOBS=2 cargo
  +1.98.1 test --locked --offline -p improvement-engine-runner`: passed,
  8 unit tests + 14 CLI E2E tests + 2 proposal-output tests (24/24).
- `CARGO_TARGET_DIR=.target-p4-proposal-activity CARGO_BUILD_JOBS=2 cargo
  +1.98.1 clippy --locked --offline -p improvement-engine-runner --all-targets
  -- -D warnings`: exit 0.

The first package test attempt correctly exposed two existing CLI assertions
that assumed the holdout was the last persisted event. They were updated to
select the holdout by stage and assert the new proposal event follows it; the
final package run above is green.

## Scope and limitations

Files changed: `crates/runner/src/main.rs`,
`crates/runner/tests/e0_proposal_output.rs`,
`crates/runner/tests/cli_e2e.rs`,
`docs/data/e0-proposal-assembly.md`,
`docs/local-e0-e2e-runner.md`, and
`docs/IMPLEMENTATION_STATUS.md`, and this journal entry. The workspace
coordination and technical bitacoras were also appended outside the worktree.

This slice only adds local persisted activity visibility. It does not publish
proposal seeds to Agent Core, authorize execution, add a Core/platform event
contract, or establish production observability. The focused runner tests,
runner all-target Clippy, formatting, and diff check are green. Full workspace
CI and fresh actual-data E2E have not been run for this slice.
