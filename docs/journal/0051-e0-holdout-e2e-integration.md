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

Independent adversarial review is pending. The code is locally committed only
after the integration feature and any agreed E2E wrapper consolidation are
stable; no claim of review approval is made here.
