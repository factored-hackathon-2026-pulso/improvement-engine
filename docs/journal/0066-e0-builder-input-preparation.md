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

## Candidate explanation projection

`project_assembled_e0_result` adds an aggregate-only explanation of the
assembled in-memory runner result. It validates candidate/run/snapshot/cutoff
consistency, selected signal binding, holdout-to-pattern and source commitment
association, and status-specific holdout counts before rendering. The contract
labels itself `assembled_in_memory_result_projection`; it is not an
independent reread or cryptographic verification of durable receipts.

The read model excludes customer/tenant identifiers, query or prompt text,
and model-generated rationale. It explicitly reports no provider invocation,
no native Agent Core proposal/evaluation, null business lift, and
non-executable status. Missing/mismatched holdout association fails closed.
Tests cover unsupported/no-candidate cases, malformed status/association, and
the successful runner persistence path. Full validation on the exact merged
main tree remains pending.

### Adversarial lineage hardening

Follow-up validation requires holdout `candidate_ref` to match the selected
signal's pattern reference and the discovery-source commitment to match the run
manifest digest. Status-specific aggregate counts, safe-rate presence, and
suppression rules are checked before projection. A no-candidate result with a
normal `insufficient_evidence` status remains distinct from malformed input.
The function name and output kind explicitly describe an assembled in-memory
projection; this code does not independently reread or re-verify durable
receipt commitments. Focused E0 explanation tests, both selected/no-candidate
CLI paths, runner Clippy, formatting, and diff checks passed in the implementing
worktree. The consolidated full local preflight also passed after porting to
the exact merged-main tree through PR #85 (`a4ccb0a84e3048b3b0b01c80bc803eba0be4615b`,
tree `1e9bdf8fa0e6bac9bf193fd1ff7eb602ff1fdd53`); PR #85 changed only the
separate debug-console subtree.

Final source rerun: `output/e0-current-main-explained-2026-10-04/run_4640_1791074275102278600`;
200 discovery cases, 154/200 recurring-query cases, 1,433/1,539 descriptive-only
holdout matches, one unlinked candidate, U20/U20-E unavailable blockers,
provider/Core invocation false, Core evaluation `not_evaluated`, business lift
null, and non-executable output. No hosted Actions result is claimed.
