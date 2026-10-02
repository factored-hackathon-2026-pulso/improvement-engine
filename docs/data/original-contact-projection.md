# Original contacts/PQR aggregate projection

This first original-source projector reads partitioned `call_center_interactions`
and `complaints` CSVs as streams and publishes only monthly aggregates. It is an
exploratory measurement input for discovery; it is not a causal model, an
operational SLA calculator, or a technical-error detector.

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

Small category/channel cells are suppressed using `--min-contact-cell-count`
(default 5, configurable from 5 to 10,000). Policy version 1 and the exact
threshold are committed into the immutable prepared-source manifest. Output
reports included count, rejected-row count, and suppressed-cell count without
revealing suppressed values or their counts. This is a technical small-cell
disclosure control, not a formal anonymity or legal guarantee.

The local motor can consume and report this descriptive projection, but this
slice does not calculate repeat-contact rates, PQR/SLA measures, technical
errors, causal associations, or an improvement candidate/proposal. Such outputs
remain unsupported rather than being inferred from contact volume. In
particular, `customer_id` is not used to calculate recurrence.

## Projection contract

| Source | Grouping | Aggregates |
| --- | --- | --- |
| `call_center_interactions` | `interaction_date` month × normalized `reason_category` (fallback to `contact_reason` only if that column is absent) × normalized `channel` | Contact count; known/positive counts for `was_resolved`, `requires_followup`, `was_escalated`; mean `duration_seconds` and `wait_time_seconds` where valid values exist |
| `complaints` | `creation_date` month × normalized `category` × normalized `reception_channel` | Creation-cohort PQR count; final-extract known/positive counts for `sla_breached`; mean `final_first_response_elapsed_days`; mean final `resolution_days` and `resolution_satisfaction` where valid values exist |

Grouping labels are closed enums. Recognized Spanish/English spellings map to
stable lower-case labels; any unrecognized, null, or PII-like category maps to
`unclassified`. Channel values map through a closed list; an absent/blank
channel rejects that row and increments `rejected_rows` rather than mapping it
to `other`. Only non-empty unrecognized channel values map to `other`. Raw
values are never stored in result types or error details.
`subcategory`, `contact_reason` free text when `reason_category` exists,
descriptions, IDs, product/customer/agent attributes, claims, compensation,
and transcripts are not read into the projection.

Event/creation timestamps are accepted only as exact UTC second timestamps
(`YYYY-MM-DDTHH:MM:SSZ`). Naive timestamps, offsets, fractional seconds, and
date-only values are rejected; the projector never guesses a timezone or
silently drops time precision. Aggregation period is the month from that
event/creation timestamp, not a claim about when the bank learned the fact.
Invalid/missing timestamps are counted as rejected, not emitted. If no valid
timezone-qualified timestamps remain, status is `unsupported` and no
aggregates are emitted.
Boolean values accept `true/false`, `1/0`, `yes/no`; other values are missing.
Durations and resolution days must be finite and non-negative. Satisfaction is
included only on the assumed common 1–5 scale; confirm the scale against the
source dictionary before interpreting its magnitude. First response time is
elapsed days between `creation_date` and `first_response_date`, at second
precision, only when both timestamps are valid and ordered; it is not a
business-hours or legally defined SLA calculation. Complaint outputs rename
these fields with `final_` prefixes and declare
`CreationCohortWithFinalOutcomes` temporal semantics. Means use only rows with valid values, with no
imputation. Every numeric and boolean metric carries `valid_count` and
`missing_count` per visible aggregate cell; their sum is that cell's row
denominator. Boolean metrics also expose positive count, and numeric means use
only valid values. Missing values never silently become false or zero.

Cells below the versioned `minimum_cell_count` policy are suppressed (default
k=5 for local smoke); `suppressed_count` reports omitted cells without
revealing their contents or counts. This is a technical disclosure-control
heuristic, not a formal anonymity or legal guarantee. The policy version and
threshold are included in the projection manifest digest.

## Availability and interpretation

The local original-bank runner uses the snapshot-count projection above. The
monthly event-time projector below remains a separate API and is unsupported
for the supplied naive-timestamp history until an explicit timezone contract
exists.

The projection is `unsupported` and emits no aggregates if any input partition
lacks a required grouping/date field, no partition is provided, or no tracked
metric field is present in every partition. `available_metrics` is the
intersection of fields present across all partitions; `missing_metrics` names
fields absent in at least one partition. A metric denominator remains its
explicit denominator; nulls do not become false/zero. Metric columns are
optional; when absent, their per-cell metric is fully missing rather than
causing fabricated values. An empty/unknown category is not evidence of a new
business reason.

## Provenance, cutoff, and coverage

Each projection requires the existing immutable `ArtifactReference` to the
source snapshot plus that snapshot's canonical byte binding, a cutoff, and an
exact inventory of opaque partition IDs with SHA-256 digests. The projector
fails closed on missing/extra/duplicate IDs, digest mismatch, duplicate
headers, malformed/truncated records, or invalid manifest. Rows after the
cutoff are excluded by exact UTC second comparison. The cutoff must use the
same exact timestamp grammar; malformed or higher-precision cutoffs are
rejected rather than rounded.

The call-center source contract declares `interaction_date` as `timestamp`
without a timezone, and the supplied contact values are naive. There is not
yet a canonical Complaints SourceContract; its dictionary lists
`creation_date`, `first_response_date`, `resolution_date`, and
`closing_date` as timestamps, but provides no timezone. The CSV header confirms
those columns exist; their availability does not establish when the outcomes
became observable relative to the UTC snapshot cutoff. Therefore complaint
rows are a cohort selected by `creation_date <= observed_cutoff`, while
`final_sla_breached`, `final_first_response_elapsed_days`,
`final_resolution_days`, and `final_resolution_satisfaction` are retrospective
final-extract outcomes that may occur after that cutoff. They are not as-of
metrics and must not be used for online/as-of decisions or leakage-sensitive
evaluation. No reliable outcome censoring is attempted until a timezone/same-
clock contract exists. All current naive timestamps remain fail-closed, so the
original-source projection emits no business aggregates.

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

The smoke test verifies the current source is reported as unsupported (naive
timestamp semantics), counts rejected rows, and emits zero aggregate cells.
It prints only partition/rejection/cell counts and the partial-coverage label;
it prints no row, identifier, category source string, path, or metric value.
This is a fail-closed compatibility check, not a successful business projection
or prevalence estimate. Historical source files remain outside Git.
