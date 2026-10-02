# 0051 — U07 durable V2 run-event ledger

## Objective

Implement the first PostgreSQL durability boundary for V2 `pulso_jobs` and
`pulso_run_events`, preserving tenant/run identity and transaction ordering.
Do not conflate the existing in-memory U07 job/timestamp projection with V2's
run/sequence ledger.

## Decisions

- Added the V2-named tables rather than a separate debug/activity table.
- Root job row lock allocates `sequence`; one transaction changes child job
  status, updates root `last_event_sequence`, and inserts the event.
- Status edges are bounded by V2 §15. This primitive does not imply quota,
  lease, or fencing authority; U06 remains their gate.
- Run-level events may omit `job_ref` and still advance the run sequence.
- Caller supplies UUIDv7 identifiers; sequence and `event_at` are persisted by
  PostgreSQL.
- Added ADR 0004 documenting why this store cannot yet implement the existing
  `RunActivityReadModel` without changing its public cursor semantics.
- Source dataset schemas remain untouched; U24 HTTP/UI is out of scope.

## TDD / verification

Implementation provides typed event/status inputs, UUIDv7/code validation,
transactionally allocated sequence, CAS on expected job status, and database
FKs/checks for tenant/run consistency. A compile cycle caught and corrected a
test-fixture status/signature mismatch; formatting and diff checks passed.

`postgres_run_events` is intentionally ignored by default and requires an
isolated `PULSO_TEST_POSTGRES_URL` plus
`PULSO_ALLOW_DESTRUCTIVE_TEST_DB=1`. It tests (1) atomic status/event commit
and sequence ordering including run-level events, (2) concurrent per-run
sequence allocation, (3) rollback of job status, root sequence and event when
an injected PostgreSQL trigger rejects the insert, and (4) child-as-root/cross-run
rejection. It uses the actual
migration, not a mock or in-memory projection. The test has not yet been run
against a live PostgreSQL instance in this checkout.

An independent adversarial read-only review initially found missing
cross-run/child-as-root cases, a missing root-sequence rollback assertion, no
run-level event coverage, and an unrestricted status edge. Those findings
were addressed with targeted regressions and the V2 §15 status graph; the
reviewer rechecked and reported no remaining NO-GO. No live database was
available for that review.

## Open contract dependency

The current `RunActivityReadModel` is job scoped and orders by event time plus
event ID; V2 requires run scope and monotonic sequence. ADR 0004 leaves read
adapter/API compatibility blocked pending an explicit versioned contract.
Durable U06 admission and transition reducers are also not wired to this
ledger by this slice.
