# DJC claim-next durable job lease

**Date:** 2026-10-04  
**Owner:** CODEX  
**Base:** verified GitHub `main` `707c5b4bb6394203ba818fbfbf73b9bcc362e753`  
**Plan:** v5 work package DJC; frozen contract C-7 v1.1

## Scope

Added `claim_next_job` to `DurableJobRepository` and its in-memory reference
implementation. It selects the oldest eligible job in explicit admission order
for the requested tenant, leases queued or expired-lease jobs whose effect
state remains `NoEffect`, and skips deferred, paused, terminal, acknowledged,
and reconciliation-required jobs. A successful claim advances the attempt,
fence, and control version exactly once. Invalid scope/worker/duration,
overflowing lease expiry, and an empty queue do not mutate claim state.

The reducer serializes access through `&mut self`; it is a reference model, not
a distributed lock or process-durable queue. A future PostgreSQL adapter must
perform candidate selection and conditional lease update atomically. The six
FRZ0 claim traces are specification traces (`recorded: false`), not replayed
PostgreSQL goldens, and this slice does not claim the trace replay or adapter
conformance acceptance.

## TDD and review

- Behavioral RED: the focused integration target failed to compile with the
  expected missing `claim_next_job` method. GREEN: the reducer and frozen
  signature were implemented, then focused tests exercised FIFO even when the
  first deterministic job ID sorts lexically after the second. A deliberate
  mutation to lexical-ID ordering failed that regression as expected.
- Added regressions for two sequential workers receiving distinct jobs,
  expired-lease reclaim at the expiry boundary, an expired earlier admission
  preceding a later queued job, tenant isolation, invalid inputs, no-candidate
  no-op, deferred/paused/terminal/effect states, and checked arithmetic.
- Independent adversarial review found no P1/P2. Counter-exhaustion branches
  use `checked_add` and preflight before mutation, but cannot currently be
  reached through the public API without adding test-only mutation hooks; direct
  overflow regression coverage remains a low-priority gap. Lease-expiry
  overflow does have a public-API no-mutation regression.
- Focused command passed: `CARGO_BUILD_JOBS=1 cargo +1.98.1 test -p
  improvement-engine-core --features local-simulation --test durable_jobs`
  (26 passed). Pinned rustfmt and `git diff --check` passed. The full Windows
  `scripts/verify-local-ci.ps1` also passed on the consolidated PR tree with one
  Cargo job: Clippy; Rust unit, integration and doc targets (core: 200 passed,
  2 destructive PostgreSQL tests ignored); Python contracts (25 passed, 1
  opt-in container test skipped); fixture validation; and Pester (20 + 5).
  No hosted Actions result is used.

## Boundaries

No database, Podman/container, external LLM, Agent Core runtime, distributed
worker race, or production deployment was exercised. No business-impact claim
is made. This slice is additive to the existing consolidated Codex PR #95;
publication is pending full local CI and exact remote-tree verification.
