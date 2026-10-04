# Original contacts/PQR aggregate projection

The original-source projectors read partitioned `call_center_interactions` and
`complaints` CSVs as streams and publish only monthly aggregates. They expose
two deliberately different contracts: a UTC as-of projection for timezone-
qualified data, and a snapshot-only descriptive projection for the supplied
naive timestamp data. Neither is a causal model, an operational SLA
calculator, nor a technical-error detector.

## Local motor: supported original-contact snapshot projection

`prepare_original_bank` now supports a narrower, separate projection from the
`call_center_interactions` CSV partitions. It streams each sealed partition,
verifies that its bytes still match the source manifest, and emits counts grouped
only by normalized reason category × normalized channel. A nonblank
`reason_category` value takes precedence, then a nonblank `contact_reason` is
used; if both are blank the safe category is `unclassified`. Channel is required.
Each valid CSV record contributes one `record_count`; this is not a count of
unique `interaction_id` values because the projection deliberately does not
read or deduplicate by that identifier. Contact reason values outside a
closed Spanish/English allowlist become `unclassified`; non-empty unknown
channels become `other`; missing channels are rejected. No source row, raw
category, identifier, free-text, event timestamp, resolution/follow-up field, or
agent/customer attribute is retained in the projection.

These values mean counts in the exact prepared snapshot, not an event-date
cohort or an as-of-cutoff count. `interaction_date` is deliberately not read:
the current source contract has no timezone and does not support a safe temporal
comparison. The run records the cutoff for provenance, but it does not filter
these snapshot counts against it. An input record dated after that cutoff is
still part of the static snapshot projection; this is not evidence it was
observable at the cutoff.

Small category/channel cells are suppressed with policy version 1 and fixed
`k=5` on the discovery-facing CLI path. The policy version and threshold are
committed into the immutable prepared-source manifest; callers cannot vary `k`
between comparable discovery runs. Exact rejected-row and suppressed-cell
counts are not serialized to agent inputs or `result.json`; suppressed values
and their counts are not disclosed. This is a technical small-cell disclosure
control, not a formal anonymity or legal guarantee.

The local runner also consumes a separate snapshot-descriptive projection from
the `complaints` (PQR) table. It groups only by literal creation month,
normalized category, and normalized reception channel. `complaint_count` counts
rows that passed the required timestamp/channel grouping checks; it is not a
deduplicated count of complaint IDs. Category and channel are coarse normalized
labels, not the source's full category/subcategory taxonomy. For visible `k=5`
cells it emits complaint counts and source-provided final-extract SLA-breach,
resolution-days, and resolution-satisfaction summaries. Each metric has its
own small-denominator suppression: when a nonzero valid, missing, or (for the
binary SLA flag) positive/negative subgroup contains 1–4 rows, that metric's
counts and value are withheld and marked `suppressed_small_denominator`. This
protects a lone valid outcome hidden inside an otherwise five-row cell. It does
not derive first-response time, calculate a legal/business SLA, or interpret
final outcomes as known at the run cutoff.
The result labels the measure `final_extract_facts_only` and preserves a
complaint-specific policy ID. The `k=5` and per-metric suppression are a
technical disclosure-control heuristic, not formal anonymization, a guarantee
against linkage or complementary inference, or a legal/privacy certification.
A complaints table with no inventoried partitions is reported as `absent`; an
inventoried table with the exact v1 header but no usable timestamp/channel
grouping rows is `unsupported`; a usable source whose every aggregate cell is
suppressed is reported as `supported_no_reportable_cells`, with no complaint
evidence admitted to detection. `supported` means at least one k-qualified
aggregate is reportable. A malformed or mismatched source schema fails
preparation rather than being mislabeled as a semantic `unsupported` result.
Neither absence, unsupported coverage, nor an all-suppressed source becomes a
fabricated zero. PQR rows are never joined to contacts,
customers, or interactions. `complaint_contact_count` continues to mean only
contact rows whose own normalized reason is complaint, not PQR-table rows.

Together, the two projections make complaint-tagged contact volume and PQR
volume/outcomes independently visible; they do not establish that a given PQR
caused a contact, that one represents a repeat interaction, or that changing a
flow would improve business outcomes. `customer_id` is not used to calculate
recurrence, attribution, or impact. Technical-error rates and causal/financial
effects remain unsupported by these two projections.

## Monthly projection contracts

| Source | Grouping | Aggregates |
| --- | --- | --- |
| `call_center_interactions` (UTC as-of) | `interaction_date` month × normalized reason × normalized channel | Contact count; known/positive counts for `was_resolved`, `requires_followup`, `was_escalated`; mean durations |
| `call_center_interactions` (snapshot descriptive) | Literal source wall-clock month × normalized reason × normalized channel | Same observed fields, typed separately as final-extract descriptive values |
| `complaints` (UTC as-of) | `creation_date` month × normalized category × normalized reception channel | Creation-cohort count; final-extract SLA flags, elapsed first-response time, resolution days and satisfaction, all explicitly retrospective |
| `complaints` (snapshot descriptive) | Literal source wall-clock month × normalized category × normalized reception channel | Complaint count; final-extract SLA flags, source-provided resolution days and satisfaction; no derived first-response duration |

Grouping labels are closed enums. Recognized Spanish/English spellings map to
stable lower-case labels; any unrecognized, null, or PII-like category maps to
`unclassified`. Channel values map through a closed list; an absent/blank
channel rejects that row rather than mapping it to `other`; no exact rejected
row total is retained in the agent-facing projection. Only non-empty
unrecognized channel values map to `other`. Raw
values are never stored in result types or error details.
`subcategory`, `contact_reason` free text when `reason_category` exists,
descriptions, IDs, product/customer/agent attributes, claims, compensation,
and transcripts are not read into the projection.

The UTC as-of projection accepts only exact UTC second timestamps
(`YYYY-MM-DDTHH:MM:SSZ`). Naive timestamps, offsets, fractional seconds, and
date-only values are rejected; the projector never guesses a timezone or
silently drops time precision. It filters rows at the snapshot cutoff and
returns `Projection<T>`, which includes the applied `observed_cutoff`.

The snapshot-only descriptive projection accepts exact naive wall-clock
timestamps (`YYYY-MM-DD HH:MM:SS` or `YYYY-MM-DDTHH:MM:SS`) and groups by the
literal `YYYY-MM` text in the source. It does not convert to UTC, compare to a
cutoff, claim event-time ordering across time zones, or say when the bank
learned the fact. The distinct `SnapshotDescriptiveProjection<T>` type has no
`observed_cutoff`; its `LiteralSourceWallClockMonth` and
`FinalExtractFactsOnly` tags prevent the output being described as as-of or
online-eligible. Offsets, fractional seconds, date-only values, malformed
dates, and impossible clock values remain rejected from grouping, not emitted.
Exact rejection totals are not exposed on the public projection. If no valid
source wall-clock timestamps remain, status is `unsupported` and no aggregates
are emitted.
Boolean values accept `true/false`, `1/0`, `yes/no`; other values are missing.
Durations and resolution days must be finite and non-negative. Satisfaction is
included only on the assumed common 1–5 scale; confirm the scale against the
source dictionary before interpreting its magnitude. Only the UTC as-of
complaint projection derives elapsed first-response days, and only when both
timestamps are valid and ordered; this is not business-hours or a legally
defined SLA calculation. It labels these fields `final_` and declares
`CreationCohortWithFinalOutcomes`. The snapshot descriptive complaint output
omits that derived duration because the source has no shared-clock contract.
Means use only rows with valid values, with no imputation. Every numeric and boolean metric carries `valid_count` and
`missing_count` per visible aggregate cell; their sum is that cell's row
denominator. Boolean metrics also expose positive count, and numeric means use
only valid values. Missing values never silently become false or zero.

Discovery uses the immutable version-1 policy with k=5; public callers cannot
provide an alternate per-run threshold. Cells below that floor are suppressed.
Exact suppressed-cell and rejected-row counts are not fields on public
projection results or agent-facing outputs. This is a technical disclosure-
control heuristic, not a formal anonymity or legal guarantee. The policy
version and threshold are included in the projection manifest digest; changing
k requires a new policy version and release.

## Availability and interpretation

The local original-bank runner consumes separate snapshot-only descriptive
projections for contacts and complaints, while retaining the flat
contact-volume projection for basic counts. Each carries its own safe evidence
and policy metadata; the result distinguishes complaint-table state (`absent`,
`unsupported`, `supported_no_reportable_cells`, or `supported`) from complaint-
contact counts. `supported` in the result means at least one k-qualified PQR
aggregate is reportable; a source can be structurally usable yet have no
reportable cells. Descriptive output carries snapshot binding, coverage, and
literal-month/final-extract semantics without assigning an as-of cutoff.

The source projection is `unsupported` and emits no aggregates if any input
partition lacks a required grouping/date field, no partition is provided, or
there is no row with a valid source timestamp and nonblank channel to group.
Source-level `Supported` means at least one usable grouping row exists;
rejected rows may coexist with usable rows. The runner separately reports
`supported_no_reportable_cells` when all source groups are below `k` and
`supported` only when a k-qualified aggregate is visible. `available_metrics` is the
intersection of fields present across all partitions; `missing_metrics` names
fields absent in at least one partition. A metric denominator remains its
explicit denominator; nulls do not become false/zero. Metric columns are
optional; when absent, their per-cell metric is fully missing rather than
causing fabricated values. An empty/unknown category is not evidence of a new
business reason.

## Provenance, cutoff, and coverage

Both monthly modes require the existing immutable `ArtifactReference` to the
source snapshot, its canonical byte binding, and an exact inventory of opaque
partition IDs with SHA-256 digests. They fail closed on missing/extra/duplicate
IDs, digest mismatch, duplicate headers, malformed/truncated records, or an
invalid manifest. Only the UTC as-of mode compares source timestamps with
`observed_cutoff`, using exact UTC-second precision. The snapshot-descriptive
mode retains the global snapshot artifact's UTC `observed_cutoff` only inside
the source snapshot binding; it neither compares source rows to that field nor
copies it into the result.

The call-center source contract declares `interaction_date` as `timestamp`
without a timezone, and the supplied contact values are naive. The versioned
Complaints SourceContract is dictionary-aligned; its `creation_date`,
`first_response_date`, `resolution_date`, and `closing_date` fields still have
no timezone semantics. The header confirms column presence, not when outcomes
became observable relative to the UTC snapshot cutoff. The UTC as-of complaint
projector remains fail-closed on these naive timestamps. The snapshot-
descriptive projection instead groups by literal source wall-clock month and
exposes source-provided SLA/resolution fields strictly as retrospective
final-extract facts; it does not compare outcomes to a cutoff or make
online/as-of or leakage-sensitive claims. It omits derived first-response
elapsed time because that requires a reliable shared-clock interpretation.

For call-center contacts, `EventDateCohort` means only that rows are selected
by `interaction_date`; it does not claim the attached `was_resolved`,
`requires_followup`, or `was_escalated` values were observable at that cutoff,
because those outcomes have no separate reliable event timestamp in the
available contract.

For partitioned tables, the additive optional `partition_inventory_digest`
seal must be the canonical partition-inventory digest produced by
`canonical_partition_inventory_digest`: SHA-256 over the domain separator
`pulso-source-partition-inventory-v1\0`, then each partition ID and its
`sha256:` digest in ID order, each UTF-8 value prefixed by its 64-bit
big-endian byte length. This supplemental seal never repurposes `file_digest`,
which continues to mean SHA-256 of the source object bytes for existing source
validation. The projector requires the supplemental seal for partitioned
inputs; older snapshots without it remain valid for non-partitioned consumers
but are not eligible for this projection. Each partition's actual bytes and
exact first-line header digest are checked against the inventory and the
snapshot's `header_digest`. Tests and local smoke bind the inventory to the
exact partition bytes they then project.

`coverage=complete` is valid only when the caller enumerates every partition in
the selected source snapshot. Bounded smoke/tests use explicit sample
inventories and `coverage=partial` (the established smoke selects the first
25 stable-sorted partitions per table). Partial aggregates are not full-history
prevalence estimates. The existing source snapshot reference is reused; this
projection does not introduce another snapshot entity.

`was_resolved`, `requires_followup`, `was_escalated`, durations, `sla_breached`,
first-response time, resolution days and satisfaction describe separate
observed facts. The projector does not join a PQR to a contact, infer why a
customer called, assert that escalation means failure, treat a supplied SLA
flag as a legal SLA calculation, or call any of these fields a technical
error. Associations between groups are descriptive only.

## Local validation

Synthetic contract tests run without access to historical data:

```powershell
cargo test --locked --offline -p improvement-engine-core --test original_contact_projection
```

For an explicitly requested bounded local smoke test, set
`PULSO_ORIGINAL_DATA_ROOT` to the directory containing both partition
directories and run. It reads the first 25 CSV partitions per table in stable
path order (not the full history):

```powershell
cargo test --locked --offline -p improvement-engine-core --test original_contact_projection local_original_contacts_and_complaints_smoke_aggregates_only -- --ignored --nocapture
```

The smoke test verifies both behaviors: the UTC as-of projector remains
unsupported on naive timestamps, while the snapshot-descriptive projector
produces disclosure-controlled aggregates for valid literal source months. The
sample has partial coverage and is not a full-history prevalence estimate. It
prints bounded partition counts, visible k-qualified aggregate totals/cell
counts and coverage; exact rejected-row/suppressed-cell totals are neither
fields on public projection types nor printed. It prints no row, identifier,
category source string, path, or sub-k metric. This is descriptive discovery
evidence, not a point-in-time or online signal. Historical source files remain
outside Git.
