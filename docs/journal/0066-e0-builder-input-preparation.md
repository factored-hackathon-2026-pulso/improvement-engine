# E0 builder-input preparation status boundary

**Date:** 2026-10-03
**Owner:** CODEX
**Branch:** `feat/e0-review-proposal-persistence`
**Base:** `0b0c919ddbc517faceda4a269af944f7d75a0c12` (verified `origin/main`)

## Scope and behavior

Added an E0-only typed runner status after `proposal_assembly`. The receipt
separates source evidence binding from builder readiness. For qualifying
candidates, the runner checks source run ID, snapshot ID/revision/digest,
tenant-to-snapshot binding, cutoff, and candidate-to-assembly provenance;
records only safe metric IDs and signal/summary digests; and reports
`evidence_binding=bound`, `status=dependency_blocked`, and the two exact local
composition gaps (`u20_plan` and `e0_safety_oracle`). No provider is called,
no Flow mapping is fabricated, and no output is named as a proposal/Core
artifact or treated as execution/evaluation authority. No-candidate E0 output
is explicitly `not_applicable`; OriginalBank receives neither this object nor
an event. The status object and event are persisted in parity to result JSON
and NDJSON.

The local runner does not compose actual U20/U20-E objects. Consequently no
`ready` state is available from this integration path. The positive test means
candidate evidence is bound, not that a builder is ready. The synthetic E0
Parquet fixture exercises the actual local CLI pipeline; no live provider or
actual-source smoke was run for this slice.

## TDD and validation

- Initial targeted attempt revealed the runner needed a direct `serde` derive
  dependency; this was setup failure, not behavioral RED. Added the minimal
  dependency and lockfile entry.
- Behavioral RED: temporarily omitted only the result JSON persistence
  assignment while retaining the new assertion. Command
  `cargo +1.98.1 test --offline -p improvement-engine-runner --test e0_proposal_output e0_cli_persists_every_qualifying_proposal_seed_with_truthful_provenance -- --exact`
  failed at the expected missing `e0_builder_input_preparation` field (test
  failure after successful compile). Restored the persistence assignment.
- An initial unit-test compile caught a shadowed test helper name; renamed the
  local variables before final validation.
- Final GREEN:
  - `cargo +1.98.1 test --locked -p improvement-engine-runner --bin improvement-engine e0_builder_input_preparation::tests` — 5/5.
  - `cargo +1.98.1 test --locked -p improvement-engine-runner --test e0_proposal_output` — 2/2.
  - `cargo +1.98.1 test --locked -p improvement-engine-runner --test cli_e2e` — 14/14.
  - `cargo +1.98.1 clippy --locked -p improvement-engine-runner --all-targets -- -D warnings` — passed.
  - `cargo +1.98.1 fmt --all -- --check` and `git diff --check` — passed.

Independent adversarial review accepted the slice with no actionable findings.
The reviewer confirmed candidate provenance checks, tenant/snapshot binding,
privacy-safe serialization, truthful U20/U20-E dependency blockers, and
OriginalBank isolation. The review was read-only and did not run Cargo; this
slice's runtime evidence remains the focused local test suite above, using a
synthetic E0 fixture rather than a new actual-source smoke. No provider
invocation or Core proposal/evaluation is implemented here.

## Current-main integration follow-up

After PR #83 merged, the E0 preparation commit was selectively ported onto
current `main` (`594be4e`). The integrated persistence regression initially
failed because its assertion treated the complete multi-line NDJSON stream as
one JSON object (`trailing characters`, line 2). The test now parses every
line and asserts that the NDJSON event sequence equals the `result.json`
timeline, including `e0_builder_input_preparation`. The focused test is green
on the integrated branch.

An actual-source sample run on the same feature code completed 200 discovery
cases and persisted one candidate. The receipt reports
`evidence_binding=bound`, `status=dependency_blocked`,
`provider_invoked=false`, and `executable=false`; recurring and selected
holdout evidence remain descriptive-only. No provider or native Core proposal
was invoked. The run artifact is recorded in `docs/IMPLEMENTATION_STATUS.md`.
