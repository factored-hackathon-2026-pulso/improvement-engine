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
    └── one PreparedSource (same immutable package; phase-tagged cases/facts)
            ├── Arranque projection ──> core discovery/simulation
            │                              │
            │                              └── selected SignalSummary.pattern_ref
            │                                             │
            │                           attest using Arranque facts only
            │                                             │
            └── Reproduccion projection ──> post-selection holdout evaluator
                                                          │
                                                          └── aggregate safe result
```

1. The runner builds `LocalRunInput` from Arranque cases and their facts only;
   `Reproduccion` events and queries are not passed to core discovery.
2. Core completes signal selection, simulated candidate admission, draft, and
   evaluation. Only after this call returns does the runner consider holdout.
3. The runner requires an E0 source, a selected primary recurrence signal with
   `pattern_ref`, and an admitted `opportunity` candidate. If any condition is
   false, it persists `e0_recurrence_holdout: null` and emits no holdout event.
4. For a selected recurrence, call
   `attest_selected_e0_recurrence_candidate(discovery_source, pattern_ref,
   minimum_arranque_support)`. The supplied discovery source must be E0 and
   have projected `copilot_query` evidence.
5. Call `evaluate_e0_recurrence_holdout(token, prepared_source, policy)` only
   after selection. It reads only Reproduccion facts from that same prepared
   source; tenant scope must match, and wrong source kind/scope are errors.
   Missing `copilot_query` in the Reproduccion projection is a valid
   `unavailable` result, not a zero-support result.
6. Persist/emit only `E0HoldoutEvaluation`. Do not serialize the candidate
   token. The result contains policy id/version/threshold, selected candidate
   ref, discovery and holdout manifest commitments, safe status, aggregate
   counts/rate, and a fixed interpretation string. The runner appends a
   `e0_recurrence_holdout` event after the core timeline, with the status and
   aggregate matching/queried counts only when the policy support floor is
   met; insufficient-support events carry no exact counts.

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
  positive matching support is below minimum. All exact counts and the rate
  are absent in this state; the top-level excluded-replay count is never
  exported for E0, including when evaluation is absent or unavailable, so it
  cannot reconstruct the hidden population. `insufficient_support` is not a
  license to expose small-cell values.
- `unavailable`: the holdout has no projected `copilot_query` table. Counts and
  rate remain absent, so absence is not misreported as a measured zero.

The present policy is `e0_recurrence_holdout` version 2, with configurable
minimum distinct-case support in the safe aggregate range 5–5,000. The CLI
accepts the same lower bound and defaults to 20; the Windows wrapper also
defaults to 20. Below the configured floor the evaluator suppresses all exact
holdout counts and rates, including the total Reproduccion count. Changes to
the policy's threshold contract or semantics must increment its version.

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
its support. `crates/runner/tests/cli_e2e.rs` additionally verifies that the
runner emits holdout only when a core opportunity candidate was selected,
places it after the discovery timeline, and changing only Reproduccion query
signatures changes the holdout status but not Arranque signal counts, proposal
hypothesis, or candidate count. Holdout output is never an input to core.

## Current augmented-sample smoke

On the local augmented E0 package, the runner selected 200 Arranque cases and
excluded 1,800 Reproduccion cases from discovery. The discovery recurrence
signal was 154/200 cases. Post-selection holdout had 1,539 queried Reproduccion
cases, of which 1,433 shared the selected opaque signature (9,311 basis points,
or 93.11%); status was `replicated` under the 20-case policy. These are
descriptive package-specific aggregates only. They do not show causality,
customer outcomes, resolution, automation success, or business value. The
rate applies only to cases with a projected query, not all 1,800 holdout cases.
It also depends on the source package's query-signature normalization; it is
not a semantic comparison of raw requests.
