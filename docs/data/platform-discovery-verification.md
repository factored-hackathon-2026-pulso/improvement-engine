# U30 Platform Scout independent verification

## Scope

`PlatformIndependentVerifier` is a deterministic, bounded verifier for the
existing U30 measured-signal → platform Scout candidate path. It verifies one
current window against at most one independently obtained, sealed U30
comparison window. It is **not** the complete U14 hypothesis verifier and does
not authorize a candidate for the generic U13-A/U14 pipeline.

The verifier checks, in order:

1. The candidate and current U30 input commitments are valid and all candidate
   measurement/source fields exactly match that input.
2. Tenant, job, grant, and authority match the invocation scope.
3. The typed Core receipt and model receipt are successful and match the
   candidate's exact input, attempt, binding/policy/capability, run, and output
   commitments.
4. A composition-root supplied, immutable metric policy matches the metric,
   version, layer, population, and trusted mapping digest. Policy configuration
   is not sourced from the model or candidate. Its minimum coverage must be in
   `1..=10_000` basis points; minimum denominator and minimum directional
   change must also be positive.
5. Current and comparison numerators, denominators, missing counts, coverage,
   as-of clocks, and windows are internally consistent. Windows may touch at
   an endpoint but may not overlap. Comparison input can be created only from
   a sealed `PlatformDiscoveryInput`, not free-form counts.
6. The current rate change is interpreted only in the explicit policy's
   direction-of-concern and uncertainty threshold.

## Result semantics

| Outcome | Meaning |
| --- | --- |
| `supported_descriptive` | The measured rate changed in the configured direction by at least the configured minimum, with both windows meeting denominator and coverage floors. |
| `refuted` | The measured rate changed in the opposite direction by at least the configured minimum. |
| `uncertain` | A comparison is missing/overlapping/not comparable, or change is within the uncertainty band. |
| `insufficient_evidence` | Current or comparison support, coverage, or clock integrity fails the configured evidence floor. |

Invalid or mismatched candidate, scope, receipt, or policy returns a typed
verification error rather than a favorable report. A zero-coverage policy is
rejected at construction, so empty observation sets cannot produce a
directional result.

The report contains candidate and policy digests, safe aggregate rates, and a
`comparison_evidence` reference carrying metric/version/layer/population,
counts, mapping and resolution digests, source-signal digest, window bounds,
and `received_as_of_ms`. Its evidence digest commits all those fields and the
comparison tenant internally; tenant/job/grant/authority identifiers are not
serialized. This lets a local auditor replay the exact comparison evidence
without exposing tenant identity in the report.

## Explicit limitations

- “Supported” means only that a policy-bound descriptive aggregate trend is
  present. It does not establish why the rate moved, customer harm, an
  intervention effect, savings, revenue, NPS, or causal/business lift.
- The current Scout candidate intentionally does not retain the model's prose.
  The verifier therefore checks the structured measurement against a
  composition-root policy; it does not claim to validate arbitrary LLM text.
- One comparison is not a full statistical analysis: there is no confidence
  interval, seasonality adjustment, multiple-window trend, stratified
  confounder analysis, or native Jev/U14 report in this slice.
- The policy is a reviewed composition-root input. Its digest proves which
  policy the report used, not that an operator selected an appropriate policy.
- Typed Core/model receipts are checked for exact binding and success; this
  Rust seam does not independently re-verify upstream cryptographic audit
  chains. The adapter that creates typed receipts remains responsible for
  authenticating them.
- A report carries no capability for opportunity creation, proposal building,
  Core registry mutation, publication, release, or execution.

## Test contract

The unit tests cover supported/refuted/uncertain/insufficient results,
cross-tenant invocation and comparison, candidate tampering, Core/model
receipt substitution, mapping mismatch, denominator and coverage floors,
missing and overlapping windows, safe report serialization, exact comparison
window/as-of provenance, and rejection of zero-coverage policy.
