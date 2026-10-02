# E0 recurrence holdout contract

## Purpose and boundary

This local-only evaluator checks whether an already-selected Arranque query
signature appears across distinct Reproduccion cases. It does not discover a
candidate, consume evaluator labels/outcomes, or estimate causal/business
impact. It reads only the source adapter's opaque query signatures and case
ordinals. Raw query text and identifiers never enter its API or output.

The candidate token is an in-memory capability, deliberately non-serializable
and non-debuggable. It binds the selected core `pattern_ref` to the discovery
manifest, tenant, opaque signature, and distinct Arranque support. The token
must be obtained with `attest_selected_e0_recurrence_candidate` after the core
runner has selected a candidate. That function recomputes the same top-
signature choice over Arranque facts only and rejects a mismatched core ref.

## Integration sequence

```text
E0 adapter
    │
    ├── Arranque AgentInputSet ──> core discovery/simulation
    │                                   │
    │                                   └── selected SignalSummary.pattern_ref
    │                                                  │
    │                           attest against Arranque only
    │                                                  │
    └── separate Reproduccion package ─────────> holdout evaluator
                                                       │
                                                       └── aggregate safe result
```

1. Keep the existing discovery call restricted to Arranque; do not pass this
   evaluator into candidate generation or discovery scoring.
2. If core selects no recurrence candidate, do not call the attestation or
   holdout APIs.
3. For a selected recurrence, call
   `attest_selected_e0_recurrence_candidate(discovery_source, pattern_ref,
   minimum_arranque_support)`. The supplied discovery source must be E0 and
   have projected `copilot_query` evidence.
4. Call `evaluate_e0_recurrence_holdout(token, holdout_source, policy)` only
   after selection. Tenant scope must match; wrong source kind and scope are
   errors. Missing `copilot_query` in the holdout is a valid `unavailable`
   result, not a zero-support result.
5. Persist/emit only `E0HoldoutEvaluation`. Do not serialize the candidate
   token. The result contains policy id/version/threshold, selected candidate
   ref, discovery and holdout manifest commitments, safe status, aggregate
   counts/rate, and a fixed interpretation string.

## Metric and status semantics

- `reproduction_case_count`: distinct cases assigned to Reproduccion by the
  prepared source, whether or not a query exists.
- `queried_case_count`: distinct Reproduccion cases with at least one projected
  query; this is the rate denominator.
- `matching_case_count`: distinct queried Reproduccion cases with one or more
  queries whose opaque signature equals the selected Arranque signature.
- `recurrence_rate_basis_points`: floor(`matching / queried * 10,000`); absent
  when the denominator is zero.
- `replicated`: denominator and matching distinct-case support both meet the
  versioned minimum.
- `not_observed`: denominator meets the minimum and no queried case matches.
- `insufficient_support`: the queried denominator is below minimum, or a
  positive matching support is below minimum.
- `unavailable`: the holdout has no projected `copilot_query` table. Counts and
  rate remain absent, so absence is not misreported as a measured zero.

The present policy is `e0_recurrence_holdout` version 1, with a configurable
minimum distinct-case support in the safe range 1–5,000. Changes in semantics
must increment its version.

## Evidence limitations

This is descriptive replication only. A replicated signature does not prove a
customer problem, causality, resolution, automation success, customer outcome,
or business lift. It is not a test/control comparison. `pattern_ref` currently
commits to the full prepared-source manifest, including Reproduccion bytes;
therefore changing holdout bytes can change provenance-derived candidate refs
even though the top Arranque signature and its support are unchanged. Treat
that as provenance binding, not statistical influence. A future decoupling of
discovery identity from full-source provenance belongs in the core contract,
not in this evaluator.

## Test obligations

`crates/source-adapters/tests/e0_holdout.rs` verifies distinct-case counting,
safe serialization, absent-vs-zero distinction, support boundaries, tenant
scope rejection, rejection of refs not backed by Arranque, and that changed
Reproduccion signatures do not alter the Arranque-selected opaque pattern or
its support. Integration tests should additionally prove the runner calls the
validator only after candidate selection and never feeds its result back into
discovery; runner integration is intentionally outside this slice.
