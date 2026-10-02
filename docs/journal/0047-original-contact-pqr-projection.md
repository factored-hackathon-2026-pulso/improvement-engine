# Journal 0047 — original contacts and PQR projector

## Decision and behavior

Adversarial hardening resolves the source `ArtifactReference` from the
immutable `ArtifactRepository`, checks exact reference and `SourceSnapshot`
kind, and reparses its stored raw `source_snapshot_json`. The table's source
seal must commit the exact partition inventory using
`pulso-source-partition-inventory-v1` (canonical sorted IDs and byte digests),
through the additive SourceSnapshot `partition_inventory_digest` field;
`file_digest` remains the original source-object byte digest. Snapshots without
the optional inventory seal stay valid for non-partitioned consumers, but this
projector rejects them. The source header digest must match every partition.
Missing, extra, duplicate or content-mismatched partitions fail closed.
Timestamp parsing requires exact `YYYY-MM-DDTHH:MM:SSZ` UTC timestamps and
compares the cutoff at second precision. The source contract declares a
timestamp without a timezone, and observed local source values are naive;
those rows are rejected and the projection is explicitly unsupported rather
than coerced to UTC or date-truncated. Invalid/missing contact timestamps
increment `rejected_rows`.
CSV headers/row widths are strict; duplicate headers and truncated rows fail
closed. Blank channels are rejected, not folded into `other`.

Every per-cell boolean/numeric metric carries valid and missing counts against
the cell denominator; numeric means use valid values only. Configurable
`minimum_cell_count` defaults to k=5 for smoke, is versioned in the manifest,
and small cells are suppressed with a count of suppressed cells. This is a
technical disclosure-control heuristic, not a formal anonymity guarantee.
PQR first-response time is elapsed seconds expressed as fractional days, not
business/legal SLA.

The Complaints dictionary includes `first_response_date`, `resolution_date`,
and `closing_date` timestamps, but neither its `TIMESTAMP` declarations nor
the current snapshot provide timezone semantics. The projector therefore does
not censor final PQR outcome fields against the UTC snapshot cutoff. It selects
the creation-date cohort at/before cutoff and labels outcome fields
`final_*` with `CreationCohortWithFinalOutcomes`; these are retrospective
cohort outcomes, not as-of metrics. Until a reliable timezone/same-clock
contract is defined, original naive timestamp rows remain unsupported and no
real-source business outcomes are emitted.
The call-center projection declares `EventDateCohort`, which describes its
interaction-date membership only and does not claim that row-attached resolved,
followup, or escalation status was independently known at that cutoff.

## Verification

Synthetic tests persist the exact snapshot artifact and compute its seals from
the same fixture bytes being projected; they cover unrelated refs, inventory
digest mismatch, header-seal mismatch, exact-second cutoff, invalid and naive timestamps,
quoted/escaped and multiline CSV, duplicate headers, truncated rows, blank
channel, denominators, k suppression, and PQR final-outcome cohort semantics,
including response timestamps after the cutoff. The gated
local smoke builds an in-memory source snapshot from the exact 25-partition
sample, marks it `partial`, and verifies current source timestamp semantics
fail closed (unsupported, rejected rows counted, no aggregate cells). This is
not a successful business projection and must not be used to infer prevalence.
Production ingestion must use the same canonical inventory convention and
define timezone semantics before this projector supports the original tables.
No source data is checked in or printed.

El CI remoto de PR #56 reportó tres errores de Clippy: uso de `is_none()` con
retorno temprano en parsing, comparación booleana contra `false` y ocho
argumentos en `manifest_digest`. Se corrigieron con `?`, negación directa y un
`ManifestDigestInput` privado, sin suppressions. La ejecución local
`cargo +1.98.1 clippy --locked --offline --workspace --all-targets -- -D warnings`
quedó verde. La suite dirigida de Rust también pasó después de la corrección.

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
