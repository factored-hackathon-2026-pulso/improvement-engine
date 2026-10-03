# ADR 0004: Keep durable run activity sequence-native

## Status

Accepted for the durable persistence boundary. Its former U24-consumer block
is superseded by ADR 0005, which adds a separate sequence-native U24 contract;
the existing U07 legacy cursor remains unchanged.

## Context

V2 §§5, 19 and 25 define the durable event identity as
`(tenant_id, run_ref, sequence)`. The sequence is allocated while locking the
root `pulso_jobs` row and is committed in the same PostgreSQL transaction as
the job state transition and `pulso_run_events` insert. U07's current
`RunActivityReadModel`, however, pages one `job_id` by
`(occurred_at_unix_seconds, event_id)` and records a per-job projection
revision. A job is not necessarily a run root, event timestamps are not the
run's serialization order, and an arbitrary event identifier is not the
sequence. Treating these values as aliases would lose hierarchy and ordering
semantics and could expose inconsistent debug history.

## Decision

- Persist V2 jobs and run events in the specified `pulso_jobs` and
  `pulso_run_events` tables; do not add a parallel activity table.
- Allocate one positive sequence per run by locking only the root job row.
- In one transaction, verify the root and child share the requested tenant/run,
  compare-and-set the child job status, advance the root sequence, and append
  the immutable event. An insertion failure rolls the entire transition back.
- Reject job status edges outside V2 §15's reducer graph. This persistence
  boundary does not grant a claim or validate quota/lease/fence authority; U06
  remains responsible for those guards and for deciding when to call it.
- Cancellation edges are deliberately absent here: V2 permits cancellation
  only after revocation and confirmation that no external effect is pending.
  That requires a separate U06 guarded operation, not a generic event append.
- A run-level event may omit `job_ref`; it advances the same root sequence but
  performs no job-status update.
- Require caller-supplied UUIDv7 IDs; the database does not silently generate
  UUIDv4 IDs that conflict with the V2 contract.
- Do not implement `RunActivityReadModel` for this store. Its job/timestamp
  cursor cannot represent V2 run/sequence continuation. U24 consumes the
  ledger only through the distinct authenticated contract in ADR 0005.

## Consequences

This provides an executable transactional persistence boundary and an isolated
PostgreSQL test target. It does not claim that current U06 reducers, job
admission, or the legacy `ActivityTimeline` are backed by this ledger. U24's
V2 read projection is defined separately in ADR 0005.

## Validation required

The isolated PostgreSQL integration test proves run-local sequence ordering
across job-level and run-level events, rejects a child as root and a job from a
different run, and proves rollback of job status and root sequence when event
append fails. CI must run it against its ephemeral PostgreSQL service with
explicit destructive-test opt-in. Unit tests validate event vocabulary,
UUIDv7 format, and rejected terminal-to-queued transition. No source dataset
table is modified.
