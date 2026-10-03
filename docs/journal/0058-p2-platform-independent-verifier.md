# Journal 0058 — P2 platform independent verifier

**Status:** implementation and correction review in progress. No PR/push.

## Goal

Add a narrow independent verifier after U30 → `PlatformScoutCandidate` that
checks the exact U30 evidence, invocation scope, Core/model receipts, a
versioned composition-root metric policy, and a non-overlapping comparison
window. It must not claim causality/lift or authorize proposal/execution.

## Implementation and adversarial corrections

- Added `PlatformIndependentVerifier` and policy/comparison/report types in
  `crates/core/src/platform_discovery_verifier.rs`; exposed the module as a
  child of `platform_discovery` so the verifier can check private candidate
  fields without making them mutable/public.
- The independent comparison is constructed from a commitment-validated
  `PlatformDiscoveryInput`; arbitrary free-form comparison counts are not
  accepted. The output carries an aggregate-only comparison reference with
  source signal, metric/mapping/resolution digests, counts, exact time window,
  and `received_as_of_ms`; its digest commits those values plus tenant scope
  while omitting the tenant from serialized output.
- The policy is required to have nonzero minimum denominator, coverage, and
  directional-change thresholds. Direction comes from that versioned policy;
  there is no inferred “bad” direction.
- TDD correction RED: the new provenance test failed to compile because
  `comparison_evidence_digest` was not implemented. Added the evidence
  reference/digest and zero-coverage policy validation.
- Earlier behavioral RED during this slice: a cross-tenant comparison was
  incorrectly labelled insufficient sample; it now returns `uncertain` with
  `comparison_tenant_mismatch`.
- Reviewer clarified that this slice is one descriptive comparison, not full
  U14; docs now state the statistical, policy-selection, and upstream receipt
  authentication limits explicitly.

## Validation

- Focused unit tests after correction: 9/9 passed with Cargo 1.98.1,
  `--locked --offline`, feature `test-support`, target `target-p2-platform-verifier`.
- First `cargo fmt --all -- --check` found formatting differences; applied
  `cargo fmt --all`. A second format check is pending.
- Focused all-target Clippy with warnings denied found only helper
  `too_many_arguments` findings; the production report helper has since been
  reduced by deriving the signed delta from the rates. Clippy rerun and
  independent adversarial re-review are pending.
- The proposal-plan lane had the serialized Cargo slot during part of this
  correction; no Cargo commands were run while it owned that slot.

## Non-claims / remaining gate

This is not full U14, an LLM prose verifier, causal inference, Core chain
verification, an opportunity/proposal flow, or execution authority. Re-run
focused tests after the last refactor, fmt check, all-target Clippy, then obtain
independent adversarial review before calling the slice ready.
