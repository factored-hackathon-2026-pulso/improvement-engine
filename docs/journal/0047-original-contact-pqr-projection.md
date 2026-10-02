# Journal 0047 — original contacts and PQR projector

## Decision and behavior

Adversarial hardening binds both contact and complaint projections to the
existing immutable source `ArtifactReference`, canonical snapshot digest,
cutoff, coverage marker, versioned suppression policy, and exact opaque
partition-ID/SHA-256 inventory. Missing, extra, duplicate or content-mismatched
partitions fail closed. Future event dates are excluded. CSV headers/row
widths are strict; duplicate headers and truncated rows fail closed. Blank
channels are rejected, not folded into `other`. Category/channel values remain
closed enums and narrative/IDs are never emitted.

Every per-cell boolean/numeric metric carries valid and missing counts against
the cell denominator; numeric means use valid values only. Configurable
`minimum_cell_count` defaults to k=5 for smoke, is versioned in the manifest,
and small cells are suppressed with a count of suppressed cells. This is a
technical disclosure-control heuristic, not a formal anonymity guarantee.
PQR first-response time is elapsed calendar days, not business/legal SLA.

## Verification

Synthetic tests cover quoted/escaped and multiline CSV, duplicate headers,
truncated rows, cutoff, exact partition/digest validation, blank channel,
denominators, k suppression, and PQR elapsed-time metrics. The gated local
smoke remains a 25-partition `partial` sample and prints aggregate diagnostics
only; it must not be used to infer full-history prevalence. No source data is
checked in or printed. This remains a projection, not a sensor/opportunity/
proposal integration; it does not infer causal relationships or technical
errors.

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
