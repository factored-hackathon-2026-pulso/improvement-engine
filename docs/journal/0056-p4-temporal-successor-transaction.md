# 0056 — P4 temporal memory successor transaction

## Scope

This slice composes a verified U23 temporal-memory use with a U05 grant lease,
the U33 receipt writer, and one durable successor-run request. It does not run
U06, create an Agent Core artifact, or claim a production grant authority.

## Contract and implementation

- The Rust entry point validates the exact U23 evidence, tenant/scope, cutoff,
  head version, and source-event timestamp before persistence.
- U05 is an injected authority port. Its lease must fence revocation until the
  callback returns after PostgreSQL commit. No configured authority returns
  `dependency_blocked`; revoked, expired, and mismatched local fixture leases
  write no receipt/job/event. The local authority is test-only and is not a
  production grant provider.
- Migration `0005_p4_temporal_successor_outbox.sql` adds the minimal source-event
  identity/digest to `pulso_jobs`, an atomic SECURITY DEFINER writer, and the
  initial `job_queued` timeline event. Exact retry yields the stored request;
  a changed immutable request conflicts; an unpaired receipt/job/event fails
  with reconciliation required. U06 state is only `requested`, not admitted.
- The successor writer accepts only engine-issued opaque event references in
  the canonical `sha256_` plus 56 lowercase-hex format. Rust validates before
  composing a durable write and the SQL function repeats the check at the
  persistence boundary; raw source identifiers and free text are rejected.
  This format check does not prove safe derivation; a future producer must use
  tenant-scoped keyed pseudonymization and never persist raw source identifiers.
- Exact-retry event lookup follows the already persisted job identity when a
  job exists. Replaying the same source event with a different requested job
  is an immutable conflict; partial receipt/job/event state remains a separate
  reconciliation-required outcome.
- The PostgreSQL integration fixture creates a least-privilege role and grants
  it execute on the atomic writer, not direct table access. Production runtime
  role bootstrap/grant remains owned by plan slice MIG0 (L-PG); this branch
  does not provision the deployed role. Because the run-event scope check is
  deferred until transaction commit, migration 0005 redefines that trigger as SECURITY DEFINER
  with fixed `search_path`; otherwise the caller's lack of `pulso_jobs` SELECT
  breaks a valid least-privilege commit.
- Every complete-pair retry invokes the U33 receipt writer before returning the
  stored successor. U33 checks the current scoped head and tombstone before its
  own idempotent receipt return, so revoking/replacing memory cannot be bypassed
  by exact outbox replay. Partial pair state is rejected before U33 writes;
  empty state invokes U33 and then persists job/event in the same transaction.

## Verification evidence

- Rust composition plus disposable PostgreSQL 17: `1 passed`; includes missing
  provider, revoked/expired/scope-mismatched lease no-write assertions, exact
  retry receipt/job/event counts, and a revocation attempt that cannot acquire
  its fence until the durable callback has returned.
- The same Rust composition/race test against disposable PostgreSQL 16:
  `1 passed`.
- External least-privilege SQL writer test against PostgreSQL 16 first failed
  at the deferred run-event scope trigger (`permission denied for pulso_jobs`);
  migration 0005 was hardened as described above and the rerun passed `1/1`.
- Added an exact-successor-replay-after-memory-revocation regression. RED on
  the previous function order: the old complete pair returned success without
  reaching U33 (`revoked snapshot cannot turn an exact replay into success`).
  An initial naive fix that called U33 before checking pair state correctly
  exposed the partial-pair ordering invariant on first admission; the final
  branch-specific ordering described above passes the same PostgreSQL 16 test.
  Latest targeted SQL integration result: `1 passed`.
- PostgreSQL 16 runtime: Codex-owned Podman machine `pulso-codex`, container
  `pulso-p4-test-postgres16-54332`, loopback `127.0.0.1:54332`, isolated DB
  `pulso_test_p4_pg16`. The pre-existing Created container/volume was preserved.
- PostgreSQL 17 exploratory runtime: same machine, container
  `pulso-p4-test-postgres-54331`, loopback `127.0.0.1:54331`, DB
  `pulso_test_p4`; this is not the pinned acceptance version.
- Focused command, repeated against each database with the same result:
  `cargo +1.98.1 test --locked --offline --target-dir .target-p4-temporal-red2 -p improvement-engine-core --lib temporal_successor_requires_u05_lease_and_commits_receipt_job_and_event_together -- --ignored --nocapture` — PostgreSQL 16: `1 passed`; PostgreSQL 17: `1 passed`.
- PostgreSQL 16 external writer command:
  `cargo +1.98.1 test --locked --offline --target-dir .target-p4-temporal-red2 -p improvement-engine-core --test postgres_memory_temporal_receipts temporal_successor_receipt_and_request_commit_atomically_and_replay_exactly -- --ignored --nocapture` — `1 passed`. It asserts runtime gets EXECUTE only, exact replay leaves one durable event, another tenant may use the same source-event ref independently, digest mismatch fails, late job failure rolls back the receipt, partial state requires reconciliation, and revoked artifact writes nothing.
- Added a second stale-replay regression using tenant B: create a valid U28 transform receipt, advance its scoped memory head through `pulso_publish_memory_revision`, then replay the original exact event. U33 rejects the stale head and the persisted receipt/job/queued event counts remain exactly `(1, 1, 1)`. This is exercised through the real governed publication path rather than mutating the head row directly.
- Latest focused PostgreSQL 16 outcomes after the stale-head case: the external writer integration above `1 passed`; Rust U05 lease/receipt/job/event composition and revocation-fence test `1 passed`. Commands used the isolated loopback DB `pulso_test_p4_pg16` on Codex-owned container `pulso-p4-test-postgres16-54332`; no other machine or DB was used.
- Final `pwsh -NoProfile -File scripts/verify-local-ci.ps1` on the frozen P4
  diff exited 0 after the stale-head regression: all 8 selected gates passed
  (pinned Rust 1.98.1, fmt, workspace Clippy `-D warnings`, Rust unit tests,
  Rust integration/doc tests, Python contracts, fixture validation, Pester).
  Core unit tests: `137 passed; 0 failed; 2 ignored`; source adapters: `2
  passed`; Rust integration/doc suites passed; Python: `11 passed; 1 skipped`
  because the explicit Podman container-test opt-in was unset; Pester files:
  `10 passed` and `5 passed`. The destructive PG cases remain explicit opt-in
  gates and were run separately below.
- Re-ran both focused P4 PostgreSQL tests after final CI against the dedicated
  PG16 database: the SQL writer/replay/rollback/stale-head integration passed
  `1/1`, and the U05 lease/atomic composition + revocation-fence test passed
  `1/1`. Server reported PostgreSQL `16.15 (Debian 16.15-1.pgdg13+2)` on
  loopback `127.0.0.1:54332`, DB `pulso_test_p4_pg16`, Codex-owned container
  `pulso-p4-test-postgres16-54332` on `pulso-codex-root`.
- Resolved local CI findings before the green run: grouped trigger identity fields in `TemporalSuccessorTrigger` to satisfy Clippy's argument limit and added narrow dead-code annotations documenting the pending executable-service composition consumer.
- `rustfmt +1.98.1 --edition 2024 --check crates/core/tests/postgres_memory_temporal_receipts.rs` and `git diff --check` passed after the stale-head case. Root's independent adversarial review of the final P4 transaction found no blocking correctness/security issue. No commit or PR has been created; integration remains pending orchestrator decision.
- Follow-up hardening on 2026-10-04: targeted Rust tests for opaque reference validation and SQLSTATE classification passed; all core library tests passed (100 passed, 2 ignored), and the PostgreSQL test target compiles. The new SQL regressions were not run because the isolated test DB URL and explicit destructive-test opt-in are absent; no Podman machine was started under host-load constraints. This is not full local CI.

## Open integration gates

1. The production U05 lease provider and executable-service composition caller
   remain explicit dependencies; this branch does not claim the actual runtime
   triggers autonomous successor creation.
2. Orchestrator decides when to commit/open the consolidated PR. Do not push or
   open a PR from this lane without that coordination.
3. Production runtime-role creation and EXECUTE grant belong to L-PG/MIG0 in
   the plan. The test-only grant is not deployment wiring; runtime invocation
   remains dependency-blocked until MIG0 provisions the configured role.

## Evidence scope correction — 2026-10-04

The PostgreSQL 16/17 focused passes and the full local CI pass above were run
against an earlier P4 revision, before the follow-up input/error/retry
hardening recorded in the last verification bullet. For the current P4 code
after commits `40f7382` and `3e486ab`, the verified evidence is: targeted Rust
unit regressions passed, the full core library suite passed (`100 passed, 0
failed, 2 ignored`), the PostgreSQL integration target compiled with
`--no-run`, and formatting/diff checks passed. The newly added SQL regressions
and complete local CI have **not** been rerun on this exact code state. Do not
reuse the earlier green PostgreSQL/CI result as a current-HEAD gate. The later
PowerShell WTR2 commit `22e857e` does not change Rust/SQL behavior.

## Local verification refresh — 2026-10-04, Codex consolidated HEAD

After the evidence-scope note above, the full non-destructive local gate was
rerun on consolidated Codex HEAD `2982f79` with `CARGO_BUILD_JOBS=1` and the
existing `target` directory. `scripts/verify-local-ci.ps1` completed and
printed `Local CI preflight passed for the selected gates.` This covers pinned
toolchain, fmt, workspace Clippy, Rust unit tests, Rust integration/doc tests,
Python contract tests, fixture validation, and the two Pester suites. Observed
counts include 139 core unit tests, 2 source-adapter tests, 52 core doc tests,
11 Python tests with one container test skipped by its explicit Podman opt-in,
and Pester 10 + 5 passing. The additional standalone WTR2 Pester file passed
2/2 when invoked directly. Destructive PostgreSQL gates and the new temporal
successor SQL regression cases were **not** run; exact-current-head PostgreSQL
behavior remains unverified. No Podman test was started.
