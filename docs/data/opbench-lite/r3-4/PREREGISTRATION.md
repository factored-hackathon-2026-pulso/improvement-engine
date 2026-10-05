# OPBENCH-lite R3-4: E0 linked operational-context preregistration

**Status:** preregistered before generating any R3-4 result tables. This is a
post-hoc descriptive extension: target themes and previously published E0
coverage/sample summaries were already known. It is not a blind or confirmatory
analysis.

**Owner:** CODEX / X-DOC
**Input snapshot:** the supplied bank dataset plus the supplied E0 platform
history sample, processed locally only.
**Analysis unit:** one E0 `case` joined by exact `complaint_id` to one bank
`complaints` row.
**Evidence scope:** exact source linkage for case-to-complaint category;
generated E0 process evidence for copilot queries and contact-close records.

## 1. Questions and permitted interpretation

This analysis describes what the E0 enrichment records for cases associated
with each bank complaint category. It will answer:

1. What fraction of E0 cases have a valid exact `case.complaint_id` →
   `complaints.complaint_id` relationship, and how many cannot be linked?
2. Among linked E0 cases with recorded copilot-query rows, how are queries
   distributed across a fixed, schema-derived query-family taxonomy, and how
   often is the same opaque query signature repeated within a case?
3. What recorded `sent_to_chat` and E0 `case_close` outcomes appear by bank
   complaint category, with their own observed/missing denominators?
4. What aggregate bank complaint status/SLA context is present by category,
   reported separately from the generated E0 process fields?

Every “why” statement is a **candidate descriptive interpretation of this
constructed E0 sample**, not an explanation of actual customer contact or bank
performance. `copilot_query` and `case_close` are generated enrichment in this
sample; they are not observed production telemetry. The exact complaint key
allows category association only. It does not make generated query/handling
behavior independently observed or causal. No p-values, confidence intervals,
effect estimates, significance claims, savings, prevented contacts, or lift
will be produced.

## 2. Preregistered inputs and exclusions

Read locally, in a single process, these sources and only the listed columns:

| Source | Allowed columns | Use and provenance |
| --- | --- | --- |
| E0 `case` | `case_id`, `complaint_id`, `opened_at`, `topic` | Exact link, cohort ordering and source coverage. E0-enriched record. |
| Bank `complaints` | `complaint_id`, `category`, `subcategory`, `status`, `sla_breached`, `resolution`, `resolution_days` | Category and separately reported bank complaint outcome context. |
| E0 `copilot_query` | `case_id`, `event_time`, `query_signature`, `tables_read`, `columns_read`, `answered_by`, `sent_to_chat` | Generated process descriptors. `query_signature` is comparison-only and is never emitted or hashed into an output identifier. |
| E0 `case_close` | `case_id`, `closed_at`, `resolved`, `contact_reason`, `resolution_code`, `csat` | Retrospective E0-enrichment outcome description only. |

Do not read or join `customer_id`, contact/transcript text, query `question_text`
or `answer`, analyst identities, `labels`, `timeline`, transaction rows,
security answers, names, account/card identifiers, or any unlisted fields.
There is no join to bank contacts or customers. No fuzzy matching, category
matching, pseudonymous-customer matching, or reconstruction of a missing key is
allowed. Original bank data and E0 source files remain outside Git and are not
sent to hosted models, external services, or reviewers.

The fields `copilot_query` and `case_close` describe generated enrichment.
Bank `complaints` fields are source facts from the supplied snapshot, but this
cross-source descriptive study does not prove that an E0 query pattern caused
or predicts a bank complaint outcome.

## 3. Cohort, join, and link grades

- Start with every `case` row in the pinned E0 sample. Expected sample coverage
  is 2,000 cases per the supplied sample documentation; actual input digests,
  schema versions, and row counts are written to an external local run manifest
  before calculation. A changed snapshot is a new, unregistered run.
- A case is `linked` only when its non-empty `complaint_id` exactly equals one
  unique bank complaint primary key. E0 `case_id` and pseudonymous
  `customer_id` are never substitutes.
- Require unique `case.case_id`, unique non-empty bank complaint keys, and at
  most one E0 case per linked complaint for the intended one-case/one-complaint
  grain. Conflicting duplicates, many-to-one joins, schema drift, or invalid
  dates fail closed before tables are generated. Null/orphan keys are counted
  only in a disclosure-safe link-coverage summary; they are not imputed.
- The R3 source-relationship field is named `source_link_grade`, with values
  `linked`, `indirect`, or `none`. This is **not** V3 `WorkflowBridge.link_grade`.
  `linked` means exact key equality. This analysis does not assign
  `same_outcome_linked`; no candidate intervention or independent outcome oracle
  is evaluated here. Any downstream improvement claim remains
  `not_evaluable` until V3 §16's bridge predicates are proven.
- No `indirect` association is used to build a case-level table. A measure
  without an exact source relation is shown as separate aggregate context or
  omitted; it is never attached to an individual case.

## 4. Frozen variables and calculations

### 4.1 Complaint category

Use the exact non-empty `category` and `subcategory` values from the linked
bank complaint row. Do not combine values after observing counts. If category
normalization is needed, use only a frozen, accent-insensitive mapping included
in the implementation and its tests; unknown values fail closed and appear
only as an aggregate unknown-coverage count if that count passes disclosure
rules.

### 4.2 Query families and repeats

For each query event, normalize `tables_read` as a sorted set against this
versioned allowlist of bank table names: `branches`,
`call_center_interactions`, `call_transcripts`, `campaign_sends`, `complaints`,
`customers`, `daily_exchange_rates`, `digital_events`, `marketing_campaigns`,
`products`, `service_agents`, and `transactions`. Assign exactly one family by
this predeclared precedence:

1. `transaction_lookup` if `transactions` is present;
2. `complaint_lookup` if `complaints` is present and rule 1 did not match;
3. `customer_or_product_context` if `customers` or `products` is present and
   rules 1–2 did not match;
4. `other_allowlisted_read` otherwise.

An unallowlisted table is a schema-drift error, not a new output label. Query
signature values and raw table/column combinations never appear in results.
`answered_by` is reduced to `freeform` or `tool`; no tool name is emitted.

- Query-family distribution denominator: linked cases with at least one
  recorded `copilot_query` row. This is not a rate of copilot adoption: a case
  with no query row is `not_observed`, not proof that no query occurred.
- A case has a **repeated query signature** if its recorded query rows include
  the exact same non-empty `query_signature` at least twice within that case.
  Repeats are computed in memory; neither signatures nor signature hashes are
  emitted.
- A case has a **repeated query family** if two or more recorded rows map to
  the same frozen family. Report both measures separately. The denominator is
  query-bearing linked cases with valid family/signature coverage.
- `sent_to_chat` is summarized over query rows with a known boolean. Do not
  treat it as a delivered, read, or successful customer response.
- No claim of query completeness is made without an explicit completeness
  contract. Missing query coverage stays unknown, not a zero-query outcome.

### 4.3 Handling outcomes

- Report E0 `case_close.resolved`, `resolution_code`, and `csat` as
  **retrospective E0-enrichment outcomes**, joined to the case by `case_id` and
  grouped by the exact linked complaint category. They are never model inputs,
  predictors, or evidence of a bank contact's real outcome. Publish known
  outcome counts separately from missing/unknown coverage; never publish a
  cohort total that could reveal a suppressed missing count.
- Report bank complaint `status`, `sla_breached`, `resolution`, and
  `resolution_days` in a separate category-level table, with each field's
  observed/missing denominator. They are complaint outcomes, not contact
  outcomes and not a measure of E0 agent performance.
- Bucket bank complaint status using the frozen domain mapping
  `Open|In Process|Escalated → open_like` and
  `Resolved|Closed → resolved_or_closed`, case-insensitively after trimming.
  Any other non-empty status is schema drift and fails closed. Do not inspect
  outcome values to invent additional buckets.
- Do not cross-tab generated query behavior against bank complaint outcomes in
  a way that implies association, prediction, or mechanism. Present the two
  evidence families side-by-side only, and mark their provenance in every
  table.
- Missing close rows, nullable outcomes, unknown categories, and invalid
  timestamps remain explicit unknown/missing states; never map them to
  unresolved, no query, no SLA breach, or zero.

## 5. Planned aggregate outputs and multiplicity

Emit only these tables, with stable row ordering:

1. `link_coverage`: E0 case count, exact-link count, null/orphan count and
   unique-key/cardinality checks. Suppress this table whenever any leaf or
   complement is below k.
2. `query_families_by_category`: linked query-bearing cases by complaint
   category/subcategory and frozen query family, with observed/missing counts,
   query-row count, `freeform`/`tool` counts, and `sent_to_chat` known counts.
3. `query_repeats_by_category`: linked query-bearing cases by category with
   repeated-signature and repeated-family case counts and their eligible
   denominators.
4. `e0_close_outcomes_by_category`: counts of known E0 `resolved=true/false`,
   known `resolution_code`, and known `csat`, for E0 cases only. This table has
   no cohort total and omits unknown counts.
5. `e0_close_coverage_by_category`: close-row, resolved, resolution-code, and
   CSAT missing/unknown counts. Apply k and complementary suppression jointly;
   if unsafe, suppress the whole table. Do not let this table's margins reveal
   hidden cells in the outcome table.
6. `bank_complaint_outcomes_by_category`: counts for the frozen bank complaint
   status buckets, `sla_breached=true/false`, resolution-known, and median
   `resolution_days` among known values. Do not emit a cohort total.
7. `bank_complaint_coverage_by_category`: unknown status/SLA and missing
   resolution/days counts. Apply k and complementary suppression jointly; if
   unsafe, suppress the whole table and ensure the bank outcome table cannot
   reveal its suppressed values.

All outputs are descriptive. There is no hypothesis test family and no
post-hoc selection or ranking. Three to five candidate “why” statements may
be selected only after tables exist, from the listed tables, and each must give
the aggregate numerator/denominator (or say suppressed), source provenance,
`source_link_grade`, and an explicit limitation. An empty evidence set or
fewer than three safe statements is reported as such; do not manufacture a
quota.

## 6. Privacy and disclosure controls

- Minimum support is `k=10` for every emitted count, rate, leaf, subtotal, and
  queryable output. Suppress a value if its numerator or denominator is below
  10, and apply complementary suppression whenever published margins could
  reconstruct a suppressed value by subtraction. Do not emit `0` for a
  suppressed value; use a fixed suppression marker with no hidden count.
- Check all table margins jointly, not only leaf cells. No overlapping release
  may reveal a suppressed cell. If safe complementary suppression cannot be
  proven, suppress the entire affected table or category slice.
- Output only aggregates and non-sensitive schema/version/digest metadata. No
  source rows, IDs, text, hashes derived from IDs/signatures, sample records,
  debug dumps, or PII canaries may occur in JSON, CSV, Markdown, exceptions,
  stdout, stderr, or logs.
- Real inputs and aggregate run outputs remain outside Git. Only synthetic
  fixtures, code, schemas, preregistration, and disclosure-reviewed aggregate
  summaries may be committed.

## 7. Reproducibility and acceptance tests

Before result generation, commit this preregistration. A run records the
preregistration commit, code/config revision, source schema versions, input
file digests, generator versions, row counts, UTC cutoff/ordering, and output
digest in an external local manifest. Input paths and source digests are not
written into published results. A second isolated run over the same bytes and
config must reproduce byte-identical aggregate artifacts.

Synthetic-only tests must cover exact 1:1 linking; null/orphan/duplicate and
many-to-one key failures; unknown categories and table names; no fallback join
through case/customer IDs; same-signature and same-family repeats; absent query
rows remaining unknown; missing close remaining unknown; post-close queries
excluded or rejected; generated-vs-bank outcome provenance separation;
`k=9` suppression and `k=10` emission; complementary suppression/differencing;
deterministic ordering/rounding; all outputs free of IDs, signatures, text and
PII canaries; and refusal to assign V3 `same_outcome_linked` from source-key
equality alone.

## 8. Commands and decision boundary

The exact local regeneration and test commands will be frozen in the R3-4
README before the first analysis run. They must require explicit paths to the
bank data root and E0 `datos` directory, write only to a new external output
directory, and refuse overwrite. A missing schema, incomplete join, unsafe
small-cell release, or non-deterministic result is a failed run, not a prompt
to change the protocol after seeing values.

No result or claim is authorized by this registration alone. Any change to
population, joins, allowed columns, family mapping, outcomes, suppression,
or question after results are viewed requires a new versioned preregistration
and must be described as exploratory.
