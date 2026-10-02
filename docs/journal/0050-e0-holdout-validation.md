# 0050 — E0 recurrence holdout validation

## Objective

Add a composable local-only evaluator for recurrence support in E0
Reproduccion, strictly after a candidate has been selected from Arranque.

## Implementation

- Added an in-memory, non-serializable selected-candidate token. Attestation
  re-derives the highest distinct-case query signature from Arranque facts and
  verifies the core pattern reference before retaining the opaque signature.
- Added a versioned support policy and aggregate-only holdout result with
  `replicated`, `not_observed`, `insufficient_support`, and `unavailable`
  statuses.
- Holdout evaluation scans only Reproduccion query facts. It distinguishes a
  missing query table from a measured zero, rejects source-kind/tenant-scope
  mismatch, and serializes no row identifiers or signatures.
- Added the integration contract and interpretation limits in
  `docs/architecture/e0-recurrence-holdout.md`.

## TDD and verification

1. Added public API tests first; initial compile failed because the holdout
   API/types did not exist (expected RED).
2. Implemented the smallest module to satisfy tests; corrected a fixture
   assumption when a first GREEN attempt showed fewer matching distinct cases
   than expected.
3. Added inclusive threshold and tenant-isolation boundary coverage.
4. `cargo +1.98.1 test --locked --offline --target-dir
   target-e0-validation-2 -p improvement-engine-source-adapters --test
   e0_holdout`: 5 passed.

## Decisions and limitations

- Reproduccion is strictly a post-selection validation input; it must not
  participate in discovery metrics or candidate generation.
- `pattern_ref` is currently derived using the full prepared manifest, so
  different Reproduccion bytes can change provenance-derived refs while the
  Arranque-selected signature/support remains the same. This slice preserves
  the existing core contract and documents the distinction.
- Recurrence is descriptive evidence only; no causal effect, outcome,
  resolution, automation success, or business lift is claimed.
- No runner/local simulation integration is included in this slice.

## Follow-up

The runner lane may invoke the API after it receives a core-selected
`SignalSummary.pattern_ref`; it must not wire the result back into discovery.
Add an E2E regression at integration time proving invocation ordering and
non-interference.
