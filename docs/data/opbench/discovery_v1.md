# OPBENCH-lite discovery preregistration v1

**Registered:** 2026-10-04 (before any OPBENCH result artifact is generated)
**Owner:** CODEX / X-DOC
**Input boundary:** local bank dataset tables `call_center_interactions`,
`complaints`, `satisfaction_surveys`; E0 operational `case` and
`copilot_query` only. `transactions`, `digital_events`, E0 `labels`,
`timeline`, `case_close`, transcripts, and all other tables are excluded.
**Purpose:** deterministic descriptive discovery and independent replication;
not causal attribution, customer-level reporting, legal SLA certification,
production-outcome estimation, or proof of bank-wide impact.

## Preregistration status and prior context

This file is the first OPBENCH-specific artifact in this branch and is
committed before `opbench-lite.json`, `cell_audit.json`, or any computed
results. Existing repository documentation and the task brief already disclose
prior aggregate claims (including approximately 41% for M3, 74.9% for M4,
20.1% for M5, and E1 154/200 with a later 93% recurrence). Those numbers are
known context, not blind discovery. They will not be used to tune cell
definitions, thresholds, or the statistical method below. OPBENCH will
recompute them from the named raw tables and record divergences honestly.

## Closed metric definitions

Rows are exact-PK deduplicated before measurement: `interaction_id`,
`complaint_id`, and `survey_id` respectively. Exact duplicate records collapse
to one record; conflicting rows with the same primary key abort the run rather
than selecting an arbitrary version. Missing/invalid metric values leave the
metric denominator; they are not converted to false, resolved, or zero.
All temporal cuts are disabled: the supplied bank event timestamps are naive
without a declared timezone. This is a final-snapshot descriptive analysis.

| ID | Numerator / denominator | Cell dimensions | Closed outcome rule |
| --- | --- | --- | --- |
| M1 `contact_unresolved_rate` | `was_resolved=false` / contacts with a parseable `was_resolved` flag | normalized `reason_category × channel`, plus overall | This is the complement of the source's first-call-resolution flag, not proof that a case was never resolved later. |
| M2 `complaint_share_of_contacts` | contacts normalized to reason `complaint` / all contacts | channel, plus overall | Complaint is a coarse source tag only; it is not a causal explanation. |
| M3 `complaint_unresolved_share_of_unresolved` | unresolved contacts with reason `complaint` / all contacts with a known unresolved flag | channel, plus overall | Same first-call unresolved definition as M1. |
| M4 `pqr_open_rate` | PQRs in `open`, `in process`, or `escalated` / all valid PQRs | normalized PQR category, plus overall | `resolved`, `closed`, and `rejected` are not open. The source status snapshot is not event-time history. |
| M5 `pqr_sla_breach_rate` | `sla_breached=true` / PQRs with a parseable SLA-breach flag | normalized PQR category, plus overall | The dictionary supplies a non-null breach flag but no SLA deadline/eligibility field; denominator means rows with an observed flag, not independently verified SLA eligibility. |
| M6 `survey_low_score_rate` | CSAT `main_score <= 2` / CSAT surveys with integer score in 1–5 | linked interaction `reason_category × survey send_channel`, plus overall linked | NPS and CES rows are excluded. A survey is linkable only by `interaction_id` to a deduplicated contact. `channel` means survey `send_channel` (not the contact's channel): Email→email, IVR→phone, App→mobile_app, Web→web; SMS and other unsupported send channels→other. Coverage is linked eligible CSAT / all eligible CSAT; unlinked rows do not enter cell rates. |
| E1 `copilot_repeat_rate` | eligible cases whose leading `query_signature` equals the discovery-half modal signature / eligible cases with at least one `copilot_query` | one overall cell | Leading query is the earliest operational query by event timestamp then stable source ordinal. Modal ties resolve by lexical opaque signature only in memory. No signature is emitted. A repeat finding is not customer intent or a causal mechanism. |

### Normalized dimensions

Contact `reason_category` is the existing engine closed vocabulary:
`complaint`, `transactional`, `technical`, `general_inquiry`, `product`,
`account`, `card`, `loan`, `other`, `unclassified`. The `complaint` aliases
are `queja`, `reclamo`, `complaint`; the unclassified bucket includes unknown
and blank values. PQR `category` uses a separate category normalizer with the
same closed output vocabulary and only the exact published aliases in the
implementation; unknown/blank labels become `unclassified`. No source label,
subcategory, description, or free text is emitted.

Contact/survey channels use the existing vocabulary:
`phone`, `web`, `chat`, `email`, `branch`, `mobile_app`, `other`. The known
Spanish/English aliases match the current source adapter; known PQR `Call
Center` maps to `phone`, while `Regulator` maps to `other`. Unknown nonblank
channels map to `other`; missing channels are excluded from channel-specific
cells but remain in eligible overall metrics where the source permits.

## Replication and discovery rule

Each bank row is assigned to discovery or replication using the first bit of
`SHA-256(UTF-8("opbench-lite:v1:bank:" + customer_id))`. The key is read and
hashed locally; the hash and customer key are never written. The bank split
does not include table name, so the same customer remains in the same half
across contact, PQR, and survey measurements. Rows with missing customer keys
remain in the overall descriptive count but are excluded from split-based
replication and its coverage denominator; no exact small count is released.
E0 has no approved customer linkage for this metric, so
`SHA-256(UTF-8("opbench-lite:v1:e0:" + case_id))` is used only to assign
operational cases to halves; neither case ID nor digest is written.

For each pre-enumerated applicable cell in the discovery half, compare its
adverse-event proportion against the pooled complement for that metric. Use a
two-sided two-proportion z test (pooled null variance). The complete planned
test family is every metric/cell pair in the fixed registry, including cells
that are inapplicable, under-supported, or non-significant. Apply
Benjamini-Hochberg FDR correction at `q <= 0.05` across all 181 planned cells,
with family size and adjusted q recorded. Cells that are overall reference
rows, inapplicable, or under-supported are assigned p=1 and remain listed in
the cell audit. This makes multiplicity cover every explored cell, not only
cells that happened to be testable. A cell is a discovery
candidate only if (a) numerator and denominator are each at least `k=10`,
(b) the comparison complement has at least 10 events and 10 non-events, and
(c) absolute rate difference is at least 0.05. These floors are fixed and not
changed after computation.

The independent replication half freezes each discovery candidate cell (and
for E1 the discovery-selected modal query signature) and recomputes its
proportion against the corresponding pooled complement. A candidate is
replicated when both comparison groups satisfy the same k=10 event/non-event
support, effect has the same direction and at least 0.05 magnitude, and the
replication two-proportion test is significant at `p < 0.05` after
Benjamini-Hochberg correction across the entire frozen candidate set. Cells
not selected in discovery do not become candidates merely because replication
looks favorable.

### Preregistration clarification (v1.1, still before any result artifact)

The original draft domain-separated bank hashes by table, which could split a
customer's contact and survey/PQR records across halves. Before any computation
or result file, v1.1 removes that table component. It also fixes the
multiplicity family to all 181 planned cells by assigning p=1 to non-testable
cells rather than shrinking the family after observing support. These are
methodological safeguards; no results were generated before this
clarification. The initial registered commit remains preserved in history.

## Status vocabulary and interpretation

- `candidate`: thresholded statistical discovery without independent
  replication.
- `candidate_descriptive`: a practically notable >=5pp effect meets support,
  but not adjusted discovery significance; explicitly hypothesis-generating.
- `corroborated`: discovery candidate independently meets replication rule.
- `corroborated_descriptive`: direction/effect replicate with support but
  replication significance is not met; never presented as confirmed signal.
- `uncertain`: support, linkage, schema, multiplicity, or direction is
  insufficient to decide. Missing evidence is not a negative finding.
- `refuted`: the pre-registered cell has adequate support in both halves, but
  fails the minimum effect or direction rule in replication. It refutes this
  specific cell/metric hypothesis only, not the broader business problem.

Three or more non-findings will be emitted from the fixed registry only when
they are supported by adequate denominators. `refuted`/`uncertain` must not be
assigned to under-supported cells; those remain uncertain/suppressed. Findings
use mechanism classes as hypothesis categories, never as proven causes.
`complaint` is only a normalized source category and is never converted into a
specific cause, technical defect, policy failure, or customer intent.

## Privacy and deterministic output

Final pack files contain aggregates only. A released count must be >=10;
otherwise it is `null` with an explicit suppression reason, and associated
rates/effects/intervals are also withheld when the support floor fails. No row
data, identifier, signature, free text, source path, or source contents are
written to the repo. Local intermediate state is process memory only. Outputs
use sorted keys, sorted cells, stable JSON formatting, and no run-time
timestamps. SHA-256 input commitments may be emitted only for source snapshot
manifests already designed for that purpose; raw file paths and row hashes are
not emitted.

## Explicit limitations

- These are measurements in a synthetic hackathon dataset, not estimates of
  real bank prevalence or customer-level causal effects.
- Contact resolution means first-call resolution (`was_resolved`); it is not
  final issue closure.
- PQR `sla_breached` eligibility cannot be audited without an SLA deadline or
  eligible-population flag; report the source limitation next to M5.
- Low-score means CSAT 1–2 on the dictionary's 1–5 scale; it is not NPS and is
  not equivalent to PQR resolution satisfaction.
- Missing linkages and unknown labels remain visible as coverage/uncertainty,
  not inferred joins or causes.
- Cross-sectional replication cannot establish temporal stability or
  transportability.

### Preregistration clarification (v1.2, still before any result artifact)

The dictionary's `satisfaction_surveys.send_channel` is the survey delivery
channel and is distinct from the linked contact channel. M6 therefore groups
by the explicitly normalized survey send channel plus the linked contact
reason. The first draft ambiguously said only “channel”; no results were
computed before this field-level definition was fixed.
