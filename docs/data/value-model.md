# Value scenario kernel (pre-ValueModel@1)

## Scope

This isolated core slice provides bounded scenario arithmetic and a deterministic
ordering aid for human/agent review. It is intentionally **not** the complete
V3 `ValueModel@1` portfolio planner and is not connected to the local E0 runner.
It does not aggregate candidate values, choose a budgeted portfolio, produce a
top-k recommendation, or prove a treatment effect. No ranked scenario should be
presented as an actionable intervention until the remaining portfolio contract
is implemented.

## Scenario arithmetic

Gross scenario value is observed burden × eligible fraction × effect if exposed
× adoption × unit value. Net subtracts implementation and operating costs. Low
gross uses the low endpoint of each benefit factor; low net also subtracts the
high cost endpoints. Base uses base endpoints. High net subtracts low costs.
Fractional products use integer basis points and cents, composing all three
rates before one conservative round-down to a cent. There are no floating point
operations.

USD amounts are signed integer cents internally and serialize as decimal
strings; ranges carry `currency: USD`. Input costs and unit values must be
nonnegative bounded amounts. Rates cannot exceed 10,000 basis points (100%).
Checked arithmetic returns an explicit error rather than wrapping.

Each factor is either a known low/base/high range or an explicit `Unknown` with
an `UnknownReason`; unknown is never converted to zero. An unknown gross input
makes gross and net unknown. Unknown costs preserve gross but make net unknown.
Confidence and implementation effort are retained for ordering and reported as
unknown factors when missing; they do not alter the monetary formula.

## Evidence and E0 boundary

Every factor carries an evidence kind and digest-shaped lineage identifiers for
run, source snapshot, and evidence, plus a phase. Empty/malformed digests,
`future_bank`, and post-selection reproduction-holdout evidence are rejected.
Observed burden must be `supplied_synthetic` from the Discovery phase. A
same-run Discovery observation (for example, a supplied 154/200 count) can be
valued as an observed scenario input. Reproduccion/holdout observations or
statuses (for example, 1,433/1,539) must not enter valuation or ordering. The
kernel never annualizes/extrapolates counts.

Digests prevent accidental exposure of raw identifiers/PII in these contracts,
but are not proof of authenticity. The source adapter must validate origin,
scope, and that the factor is bound to the intended source bytes before any
trusted E0 path uses the result. Public scenario calculation accepts assumptions
for arithmetic only; it is not an authentication/authorization boundary.
External assumptions remain explicitly labeled, with their source citation
recorded by the caller. Nothing here claims realized savings, causality, or
lift.

## Deterministic ordering (not portfolio optimization)

`ValueModelConfig` has a schema version, explicit configuration version, and a
canonical JSON SHA-256 digest. Its fixed criteria set starts with safety,
evaluability, and eligibility gates; those gates cannot be reordered. Passing
candidates are returned separately from candidates whose gates fail or are
unknown. Within each group, default order is base net scenario descending,
narrower net range, lower implementation effort, higher confidence, opaque
scope digest, then stable candidate ID. This is a review ordering—not expected
value, risk-adjusted return, or a portfolio recommendation. Missing metrics sort
after known values. Duplicate/empty IDs and non-digest scope identifiers are
rejected.

Still required before `ValueModel@1`: verified source-issued opaque evidence
capabilities (rather than caller-provided digests), exact population/episode
identity and marginal contribution proofs, safe overlap calculation, coherent
uncertainty/scope handling, budget constraints, and top-k selection. Until those
contracts exist, portfolio value remains unknown and candidates are never
summed. Incremental effect over an already-counted population is unsupported;
do not infer non-overlap from labels or metric names.

## Tests

`crates/core/tests/value_model.rs` covers fixed-point endpoints and conservative
rounding, unknown propagation and reasons, confidence/effort unknowns, invalid
ranges/rates/negative costs, bounded large arithmetic, evidence kind/phase
rejection, USD decimal/currency wire shape, unknown serde round-trip and
unknown-field rejection, config digest/gate order, safety-first deterministic
ordering, duplicate identities, scope-digest shape, and explicit deferral of
unknown gates.
