# Journal 0047 — original contacts and PQR projector

## Decision and behavior

Added a read-only streaming Rust projection over original contact and complaint
CSV partitions. It groups only by month, closed normalized category, and closed
normalized channel, and emits aggregate counts/means plus known denominators,
including elapsed calendar days from PQR creation to first response when valid.
IDs and narrative/free-text fields are not queried into result objects. Unknown
category/channel values collapse to safe enum values. Incomplete required
partition schemas yield `Unsupported` with no aggregates; metric coverage is
reported across all partitions. This is descriptive evidence, not causal
attribution or a technical-error signal.

## Verification

Synthetic tests cover aggregate values, unknown/PII-like category containment,
omitted identifiers and narrative, missing dimensions/SLA support, and metric
availability across partitions. A gated local smoke test reads 25 stable-sorted
partitions per table and asserts support/non-empty aggregate output; it prints
only record/rejection/aggregate-cell totals. No source data is checked in or
printed. This is a projector only: it is not yet wired to a sensor, opportunity,
proposal, or end-to-end run, and its bounded sample must not be used to infer
full-history prevalence.

## Trade-offs and limits

- Uses the existing core crate and a small `csv` dependency rather than a new
  service/crate; the contract is pure read-to-aggregate and can later be moved
  behind a source adapter without changing domain aggregates.
- Category allowlists are intentionally conservative: unsupported/unseen
  labels become `unclassified`, preserving privacy at the cost of detail.
- Resolution satisfaction is assumed to use 1–5; verify in the data dictionary
  before attaching business interpretation.
- Contact and PQR aggregates are deliberately not joined: the available source
  keys are excluded from the projection and a join would overstate linkage.
