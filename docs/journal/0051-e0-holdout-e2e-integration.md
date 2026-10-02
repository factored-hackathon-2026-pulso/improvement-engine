# 0051 — E0 holdout in local E2E runner

## Objective

Connect the approved E0 recurrence holdout evaluator to the local E2E runner
without allowing Reproduccion evidence to influence Arranque discovery,
candidate selection, or proposal construction.

## Implementation

- Added `evaluate_selected_recurrence_after_discovery` in the runner. It runs
  only after `run_local_simulation` has returned and requires an E0 source, a
  primary selected recurrence signal with a pattern ref, and a recorded
  opportunity candidate. No selected candidate means no holdout call/output.
- Attests the core pattern reference against Arranque, then evaluates the
  prepared source's Reproduccion query signatures with the same configured
  distinct-case threshold. The prepared source projection contains no labels,
  case-close outcomes, raw text, or customer identifiers in this path.
- Persists a top-level `e0_recurrence_holdout` object with aggregate counts,
  source/policy commitments, status, and the fixed non-causal interpretation;
  appends a safe aggregate timeline event after the core timeline.
- Updated the architecture contract with actual integration ordering and added
  end-to-end regression coverage.

## TDD and verification

1. Added CLI assertions first; expected RED because the persisted result had no
   `e0_recurrence_holdout` field.
2. Wired the post-run evaluator and safe persistence. Initial GREEN exposed a
   correct semantic boundary: one queried holdout case below the configured
   minimum is `insufficient_support`, even when its signature differs.
3. Expanded the synthetic fixture to 21 Reproduccion cases so the tests cover
   both positive replication at the threshold and `not_observed` with adequate
   denominator when only holdout signatures change.
4. Focused `cli_e2e` suite passed (3 tests), followed by the complete
   `cargo test --workspace --locked --offline` suite. Workspace Clippy with
   `--all-targets -- -D warnings` and `cargo fmt --all -- --check` passed.
5. After consolidating the reviewed Windows local-E0 wrapper branch, extended
   its allowlisted summary for holdout status and matching/queried counts.
   Pester first failed because the holdout was omitted (RED), then passed 9/9
   including absent-field and malformed-JSON no-leak cases (GREEN).

The local augmented E0 smoke completed: 200 Arranque discovery cases and
1,800 excluded Reproduccion cases; the discovery recurrence measured 154/200.
Holdout observed query signatures for 1,539 distinct cases, with 1,433 matches
(9,311 basis points) and `replicated` status at the 20-case threshold. Only
these aggregates were inspected; no row values or identifiers were emitted.

## Non-interference evidence and limitations

- E2E compares runs with identical Arranque but different Reproduccion
  signatures: Arranque numerator/denominator, proposal hypothesis, and
  candidate count remain stable; only holdout support/status changes.
- The holdout event is appended after all core events, including proposal and
  run completion. No result from the holdout evaluator is passed back into the
  core call.
- The PowerShell wrapper surfaces only an allowlisted status and aggregate
  counts with a fixed descriptive-only label. Missing field reports `none`;
  malformed JSON errors do not include parser payload or source data.
- Full-source manifest commitments and provenance-derived refs can change
  when Reproduccion bytes change. They are provenance, not discovery scores;
  compare semantic metrics rather than requiring byte-identical run identity.
- `replicated` is a descriptive recurrence finding, not evidence of causality,
  customer resolution, successful automation, or business lift.
- The observed holdout rate's denominator is queried Reproduccion cases only;
  the 261 other Reproduccion cases had no projected query and are not counted
  as non-matches. The synthetic/augmented sample and its opaque signature
  normalization do not establish production prevalence or semantic equivalence.
- No PR is opened until the independent review is complete.

## Review status

An independent adversarial review returned GO with two contract clarifications:
the runner uses one phase-tagged `PreparedSource` rather than separate package
objects, and the holdout library policy accepted a threshold of one despite the
CLI's floor of five. The architecture diagram and API description now state the
single-source phase boundary; the aggregate policy floor is five and its
version advanced to 2. A regression test was added first and confirmed RED
because threshold four was accepted, then passed after the policy change.

The reviewer reran focused source-adapter holdout tests (5/5), CLI E2E tests
(3/3), and Windows Pester tests (9/9). Root reran the real local wrapper after
the PR #54/#56 merges using a new output directory; it returned
`complete_simulated`, 200 Arranque discovery cases, 1,800 excluded
Reproduccion cases, 154/200 recurring-query support, and a descriptive-only
holdout of 1,433/1,539 queried cases. The proposal remained
`simulated_unverified` / `not_executed`, and the formal route remained
`do_nothing`. Only these allowlisted aggregates were inspected. Podman remains
unverified; this run used the native local E0 runner, not the Compose stack.

A second adversarial pass found that the threshold alone did not prevent
disclosure: the insufficient-support result still carried exact queried and
matching counts, and the PowerShell summary printed them. The fix suppressed
those fields, but a further independent privacy pass found another route:
`excluded_replay_case_count` exposed the total replay population and could
reconstruct the hidden holdout size. The first fix suppressed it only when
holdout status was insufficient; a third adversarial pass found this still
leaked when holdout evaluation was absent or unavailable. The current policy
does not export that total for any E0 run, independent of evaluation status.
The wrapper validates `source_kind`, requires a null E0 replay total, and
prints only `replay_excluded=suppressed`. Rust adapter tests pass (6/6); runner
CLI and wrapper regressions cover support 1–4, denominator below floor, and
E0 with no holdout evaluation. Final full tests and independent re-review are
pending. No final GO is claimed yet.

## Environment boundary

The engine-local Compose manifest and wrapper are in this repository, but the
Windows Podman backend smoke remains unverified because Podman returns
`Access is denied`. Structural tests and the native E0 CLI smoke do not prove
the Podman Compose stack is running; `deployment-boundary.md` and
`OPEN_GAPS.md` preserve that blocker.
