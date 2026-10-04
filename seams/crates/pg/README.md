# pg (MIG0 / PGC / PGJS)

Claude-owned Postgres seam; never depends on `crates/core`.

- `migrate`, `roles`, `schema`: migration runner (`migrations/`), role bootstrap, schema snapshot.
- `repo` + `conformance`: `JobRepository` port (C-7 v1.1 claim-next shape, `&self` so threads can race), the
  in-memory `MemRepo`, and the reusable conformance suite `conformance::run_suite` (returns failing scenarios).
  Scenarios follow `contracts/engine-steps/pack/parts/c7_claim_next` and Codex `durable_jobs.rs` semantics:
  lease reclaimable at `now >= expires`, fence and attempt +1 per claim, `effect != NoEffect` skipped, tenant
  isolation, single winner under threads, a superseded or expired worker cannot commit, one `out/N` per step.
  `tests/repo_conformance.rs` runs it on `MemRepo`, on `PgRepo`, and proves it fails on a deliberately racy repo.
- `pgrepo::PgRepo`: Postgres implementation over `pulso_jobs` (`lease_version` = fence token). Claim is one
  `UPDATE ... (SELECT ... FOR UPDATE SKIP LOCKED)`; `commit_output` is one transaction (conditional UPDATE on
  owner + fence + unexpired lease, then INSERT into `pulso_job_outputs`, primary key = single winner).
- `pgstore::PgJobStore`: the engine `JobStore` (get/cas/`commit_guarded`) over `pulso_job_kv`; `commit_guarded`
  locks the `lease` row (`FOR UPDATE`), verifies fence + unexpired lease and inserts `out/N` in one transaction.
  `engine::conformance::run_suite` and the kill -9 resume (`bin/pg_engine_run`) run against it
  (`tests/engine_on_pg.rs`). Migration `0051_pulso_job_claim_commit.sql` adds `effect_state`, `pulso_job_outputs`
  and `pulso_job_kv` (additive).

## Running the Postgres tests

    PULSO_TEST_PG_ADMIN=postgres://postgres:<password>@127.0.0.1:<port>/postgres \
    CARGO_TARGET_DIR=D:/cargo-targets/claude-seams-pgjs \
      cargo test --manifest-path seams/Cargo.toml --offline -j 2 -p pg -p engine

Each test creates and drops its own database (`mig0_<pid>_<n>`) on that server.

- `PULSO_TEST_PG_ADMIN` unset: Postgres tests print `SKIP` and pass.
- `PULSO_REQUIRE_POSTGRES=1`: an unset `PULSO_TEST_PG_ADMIN` is a failure instead of a skip (use in CI).

Known limits: the time source is the caller's injected clock, not the database clock; `PgRepo` opens a
connection per call (fine for tests, a pool is the production follow-up).
