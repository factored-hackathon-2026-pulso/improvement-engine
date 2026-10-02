# E0 operational-package validation

## Decision

Add a read-only source-adapter preflight for the nine operational tables in
`platform_history.json`: `case`, `identity_check`, `turn`, `routing_step`,
`copilot_query`, `tool_call`, `approval`, `case_close`, and `signal`.
Validation is a separate API from discovery. It checks the delivered contract
and Parquet package but does not produce a discovery projection or load
`labels.parquet` / `timeline.parquet`.

`case_close` is treated as post-contact outcome evidence: its keys and close
clock are checked, including closure after recorded interaction events, but no
resolution/CSAT values enter `PreparedSource`. `signal` is checked as an
operational table, including its time window and evidence-case references, but
is not trusted as evidence for discovery. Discovery derives candidate evidence
from interaction history; it does not consume precomputed platform signals.
The discovery manifest therefore excludes `case_close` and `signal` bytes.

`copilot_query` remains allowlisted interaction history. Its normalized
`query_signature` is already hashed before projection, as are table/column
categories; this supports a future repeated-query detector without raw query
text or customer values. This slice does not implement that detector.

## Validation behavior

- Require the nine named operational tables and every contract field; reject
  duplicate/missing columns and logical Arrow type mismatches. Preserve extra
  source columns for forward compatibility, counting them without projecting
  or exposing their values. Required means a non-null value in each row; Arrow
  nullable metadata is not treated as the data-quality guarantee.
- Check unique table keys, case foreign keys, approval/tool-call cross-links,
  signal evidence-case references, and event/close/signal-window clock order.
- Return only contract version, table names, row counts, and schema counts.
  Diagnostics identify table/column and failure category (and, for dangling
  turn evidence, only the unresolved-reference count); never include values,
  identifiers, or source rows.
- Evaluator files are not read, hashed, or counted by this validation API.

## Verification

TDD record: the initial compile/test run was RED because the new validation
API was not yet exported; after implementing the validator and fixture suite,
the source-adapters crate is GREEN. The optional real-package check is a
deliberate expected-failure assertion for the currently observed relation
mismatch, so the test suite can remain green without claiming that the package
passes validation.

Synthetic fixture verification covers all nine tables, required fields and
types, missing/duplicate columns, cardinality, relations, clocks, and the
evaluator/discovery boundary. With `PULSO_E0_SAMPLE_ROOT` set to the local E0
sample, the preflight confirms table/field/type compatibility and then rejects
the package at the declared `turn.evidence_ids[]` relation: 845 references do
not resolve to `copilot_query.query_id`, `tool_call.call_id`, or
`approval.approval_id`. The error exposes only the count. This conflicts with
the sample report's claim of zero platform-history violations; treat it as an
unresolved data/contract discrepancy, not as evidence to weaken the relation
check. The fixture suite is green; full real-package validation is currently
not green for that specific reason.

Validation proves only schema and structural consistency for the supplied
snapshot. It does not establish truth, completeness, provenance, production
representativeness, or causal meaning of generated operational events.
