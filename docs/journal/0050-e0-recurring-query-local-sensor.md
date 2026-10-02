# E0 recurring-query sensor in local E2E

## Decision

Add a descriptive recurrence metric over the existing treated
`copilot_query.query_signature` projection. The signature is used only in
memory to group cases; the output commits a digest-based opaque pattern
reference and never emits the signature, query text, identifiers, or evaluator
labels. Hashing is not anonymization and is not claimed as a formal privacy
guarantee; the local output is treated as sensitive derived data.
Support counts distinct Arranque cases, not query rows. The policy
`e0_recurring_copilot_query_support_v1` has a configurable distinct-case floor
(default 20, CLI range 5–5,000); version, threshold, source snapshot, and
pattern commitment flow into the signal/proposal provenance.

Technical error and recurrence metrics are measured independently. If both
qualify, `local_primary_signal_v2` prioritizes directly observed technical
failure over semantically opaque recurrence; the result preserves both
metrics, while the current local Scout pipeline emits one exploratory draft
for the primary signal. This is an explicit prototype limit, not a claim that
the secondary signal was resolved. An absent source table is marked
`source_table_unavailable`, not interpreted as a zero recurrence rate.

The recurrence candidate is intentionally `unclassified_candidate`,
`simulated_unverified`, and `not_executed`; formal route remains
`do_nothing`. Repeated opaque signatures do not establish intent, causality,
automation suitability, customer impact, or business lift. No LLM/provider or
Agent Core runtime is called.

## Test and review record

- RED: the new test first failed to compile because the result had no explicit
  recurrence availability status.
- GREEN: core tests cover distinct-case support despite duplicate rows,
  below-floor no-op, simultaneous qualifying metrics with stable documented
  priority, absent query table, and non-executable proposal semantics.
- CLI regression covers the real binary path from the E0 adapter through
  Arranque/cutoff filtering and recurrence aggregation; Replay query evidence
  is excluded. It also covers an absent optional query table.
- E0 package contract validation excludes `signal.parquet` and `case_close`
  from discovery source manifests. `case_close` is outcome evidence;
  `signal.parquet` is a platform-generated operational signal excluded from
  discovery to prevent precomputed results leaking into detection.
- Full workspace tests (`cargo +1.98.1 test --locked --offline --workspace`)
  and strict Clippy (`cargo +1.98.1 clippy --locked --offline --workspace
  --all-targets -- -D warnings`) passed after the policy and availability
  refinements; formatting and `git diff --check` also pass.
- Actual local CLI smoke (2026-10-02): 200 Arranque cases, 1,800 Replay cases
  excluded; recurrence support 154/200 at floor 20; technical-error evidence
  0/187 supported with 13 missing; three Scout candidates, one exploratory
  proposal, no execution or business-improvement claim. This is a positive
  path demonstration on the augmented sample, not a finding about the real
  bank. A separate package relation preflight still reports 845 unresolved
  `turn.evidence_ids[]` references; that contract/data discrepancy is not
  resolved by this local runner and remains a blocker to claiming the entire
  package passes its contract validator.

## Boundaries

This local composition is a test harness over existing engine modules. It does
not authenticate native U12-E evidence or exercise native Agent Core. The
original-bank projector remains separate and is not connected to this
detector. No data is sent to external services.
