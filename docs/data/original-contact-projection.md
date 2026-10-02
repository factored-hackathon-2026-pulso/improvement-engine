# Original contacts/PQR aggregate projection

This first original-source projector reads partitioned `call_center_interactions`
and `complaints` CSVs as streams and publishes only monthly aggregates. It is an
exploratory measurement input for discovery; it is not a causal model, an
operational SLA calculator, or a technical-error detector.

## Projection contract

| Source | Grouping | Aggregates |
| --- | --- | --- |
| `call_center_interactions` | `interaction_date` month × normalized `reason_category` (fallback to `contact_reason` only if that column is absent) × normalized `channel` | Contact count; known/positive counts for `was_resolved`, `requires_followup`, `was_escalated`; mean `duration_seconds` and `wait_time_seconds` where valid values exist |
| `complaints` | `creation_date` month × normalized `category` × normalized `reception_channel` | PQR count; known/positive counts for `sla_breached`; mean elapsed calendar days to `first_response_date`; mean `resolution_days` and `resolution_satisfaction` where valid values exist |

Grouping labels are closed enums. Recognized Spanish/English spellings map to
stable lower-case labels; any unrecognized, null, or PII-like category maps to
`unclassified`. Channel values map through a closed list; an absent/blank
channel rejects that row and increments `rejected_rows` rather than mapping it
to `other`. Only non-empty unrecognized channel values map to `other`. Raw
values are never stored in result types or error details.
`subcategory`, `contact_reason` free text when `reason_category` exists,
descriptions, IDs, product/customer/agent attributes, claims, compensation,
and transcripts are not read into the projection.

Dates must begin with a syntactically valid `YYYY-MM-DD` date. Aggregation
period is the month from that event/creation date, not a claim about when the
bank learned the fact. Invalid rows are counted as rejected, not emitted.
Boolean values accept `true/false`, `1/0`, `yes/no`; other values are missing.
Durations and resolution days must be finite and non-negative. Satisfaction is
included only on the assumed common 1–5 scale; confirm the scale against the
source dictionary before interpreting its magnitude. First response time is
elapsed calendar days between `creation_date` and `first_response_date`, only
when both dates are valid and ordered; it is not a business-hours or legally
defined SLA calculation. Means use only rows with valid values, with no
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
cutoff are excluded. Source date columns are day-granular; no event-time
precision is inferred.

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

The smoke test asserts support and non-empty aggregates and prints only
per-table record/rejection/aggregate-cell counts. It prints no row, identifier,
category source string, path, or individual metric value. Historical source
files remain outside Git.
