# DPGc durable-job repository conformance

## Scope

Added a backend-generic, sequential conformance suite over the frozen
`DurableJobRepository` port. The current `MemoryFixture` runs the shared
behavioral cases against the in-memory reference; a future repository adapter
can provide fixture construction, admission and read-only snapshots while
reusing the same conformance functions.

The suite covers FIFO claim order, two sequential workers receiving distinct
jobs, expiry-boundary reclaim, monotonic fencing and attempt/control-version
changes, invalid inputs and no-op behavior, tenant isolation, and filtering of
paused, cancelled, unknown-effect, completed, acknowledged and deferred jobs.
It also interprets all six FRZ0 C-7 JSON files as specification traces and
checks their frozen identity, pack version, `kind=specification_trace`, and
`recorded=false` provenance before replaying their steps against the in-memory
reference.

## Boundaries and evidence

The suite is sequential and its recovery operation exercises the in-memory
reducer on the same fixture. It does not prove concurrent database claims,
PostgreSQL `SKIP LOCKED`, process-restart durability, or distributed atomicity.
The FRZ0 files are specification traces, not PostgreSQL-recorded goldens. Those
stronger guarantees remain assigned to the PostgreSQL and SWC work packages.

TDD: the empty Cargo target initially ran zero tests; the first harness
invocation failed to compile because the replay harness did not exist. This was
a harness-capability RED, not a failure of the already-implemented DJC claim
behavior. After implementing the fixture adapter, semantic cases, and trace
interpreter, the focused `conformance_jobs` target passed 5/5. The final
consolidated-tree `scripts/verify-local-ci.ps1` preflight exited 0 with
`CARGO_BUILD_JOBS=1`: pinned Rust format and Clippy, 202 unit tests passed / 2
opt-in PostgreSQL tests ignored, Rust integration and doc gates passed, Python
25 passed / 1 opt-in container test skipped, default fixture validation passed
(six Spec-22 wire families remain incomplete), and Pester 20 + 5 passed.
Formatting and `git diff --check` passed.

Independent adversarial review found no P1/P2 issues. Optional P3 observations
were recorded: future hardening could reject malformed optional trace fields
and avoid matching Rust `Debug` strings; current frozen files are well-typed and
the current expected error names match. No PostgreSQL or container was started.
