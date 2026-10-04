# Replay clock protocol kernel

## Scope

Added a deterministic temporal visibility planner for ordered replay cases.
Cases with equal `opened_at` are emitted as a single cohort. An event is visible
to a cohort only if its source case opened strictly earlier and its
`event_at`, `available_at`, and (when physically observed) `ingested_at` are no
later than the cohort cutoff. Once visible, an event remains visible to every
later cohort. IDs and outputs are canonicalized for input-order-independent
results.

Callers must choose and the returned plan records one clock provenance:
`MeasuredIngestion`, which rejects a missing `ingested_at`, or the explicitly
assumed `EventTimeZeroLagAssumption` profile. This keeps absent physical clocks
from being silently represented as measured production timing.

## Boundaries

This is a replay clock kernel only. It does not execute detectors, score oracles,
update memory/configuration, drive a frozen or prequential campaign, construct
FRZ0 fixtures, or claim E0/FRZ0 replay acceptance. The profile must be selected
by the governed source adapter; this API does not prove the source's physical
schema. Cumulative visibility materialization uses O(C log C + E log E + E log
C + V log E) time and O(C + E + V) memory, where C is case/cohort count, E is
event count, and V is emitted event/cohort visibility pairs.

## Tests and review

Inline unit tests cover tied cohorts, inclusive cutoffs, future event,
availability and ingestion exclusion, persistent visibility into later
cohorts, explicit-profile behavior, duplicate/empty IDs, unknown source cases,
and deterministic output independent of input order. The expected module API
failed to compile before implementation (RED). Focused Rust tests passed 4/4.
Final `scripts/verify-local-ci.ps1` exited 0 with `CARGO_BUILD_JOBS=1`: pinned
Rust format and Clippy passed; Rust unit tests passed (204 passed, 2 destructive
PostgreSQL tests ignored); all Rust integration and doc tests passed, including
the existing C-7 target (5/5); Python contract tests, fixture validation, and
Windows Pester gates passed. One container-backed Python check remained
opt-in/skipped. No PostgreSQL, Podman, or container was started.

Independent adversarial review found and drove fixes for non-cumulative event
visibility (P1) and implicit missing-ingestion fallback (P2). Re-review found
those issues resolved and no new P1/P2. A P3 complexity-accounting note was
corrected in the code comment. No PostgreSQL or Podman service is started by
this slice.
