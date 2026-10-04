# PGLIVE: first real-Postgres run of the Postgres code paths (Team CL, wave 6)

Date 2026-10-04. One `postgres:16` container (PostgreSQL 16.15) on Podman machine `pulso-dev`
(`--cgroups=disabled --pids-limit=0`, labels `com.pulso.team=claude`, `com.pulso.namespace=pglive`, random loopback port,
throwaway generated passwords never printed or committed). Torn down at the end (see "Teardown").
Branch `claude/w6-pglive` = `claude/w6-r1m-review` + merge of `claude/w6-r1s` (clean, no conflicts).

## Results

| Step | What ran | Result |
|---|---|---|
| 1 DDL | `db/sql/001..090` applied, then `migrations/0001..0053` via psql (container client, stdin) into a fresh db; `db/sql` re-applied a second time | 0 errors, both times. raw 22 / augmented 9 / product 7 tables, schemas raw/augmented/product/pulso |
| 1 least privilege | real LOGIN as `pulso_raw_ro`, `pulso_augmented_ro`, `pulso_product_ro`, `pulso_loader` (passwords set at run time) | see below; all denials are `permission denied`, none rely on the read-only flag |
| 2 loader | `db/loader/load_parquet.py` with synthetic pyarrow Parquet for all 38 catalog tables (raw 22, augmented 9, product 7) | 37 of 38 tables x (load 3, idempotent re-run, replace same batch with 5, second batch, default content-hash batch id twice): 518 rows written in 4.1 s (`product.event_log` hit its unique `sequence` because my fixture reused sequences across batches: fixture artifact, not a loader bug); forbidden/lineage/unknown refused (9 cases); bad timestamp load rolled back and left the earlier batch intact; CLI exit 0 |
| 3 simulator sink | `product_stream --postgres` seeds 1-4: backfill 500 (0.8 s), re-run without `--overwrite` refused, `--overwrite` 300, backfill 200 + follow (740 rows), backfill 740 | `event_log.sequence` dense every time (count = max = distinct). Two bugs found (below) |
| 4 gated cargo tests | `cargo test -j 1 -p sources -p control-api -p pg`, `PULSO_TEST_PG_ADMIN` set, `PULSO_REQUIRE_POSTGRES=1` (a skip is a failure) | all pass after fix 3: pg crate 24 live tests (parity, pg_race, engine_on_pg, mig0051, runner, roles, repo_*), control-api store_conformance 17/17 (was 15/16), sources pg_live 6/6. thread10/engine have no PG-gated tests |
| 5 monitor e2e | simulator -> real `product` schema -> `PostgresProduct` as `pulso_product_ro` -> package -> `PgStore` as `pulso_app` (`pulso.source_watermark`, FOR UPDATE CAS) | 300 events, batch cap 40: first tick killed before commit (no watermark, one run record); restart on fresh connections replays the same batch onto the same run id (no second record), 8 batches, every event exactly once, watermark `seq:300`, `batches=8`; next tick Idle, row unchanged. Follow mode: ticks ran while the simulator wrote (rate 40/s), 550 events, each read exactly once, final watermark = max sequence 550, 9 s |

### Least-privilege matrix (run with a real login, `SET default_transaction_read_only=off` first)

- `raw_ro`: SELECT on a denied column (`bank_customers.first_name`, `e0_approval.params`), `SELECT *` on those tables, other schemas (`augmented`, `product`, `pulso`): permission denied. Granted columns readable. INSERT/UPDATE/DELETE/TRUNCATE, CREATE TABLE in `raw` and `public`: permission denied even with read-only switched off.
- `augmented_ro`: `turns.text`, `tool_calls.params`, `SELECT *`, `raw`, `pulso`: denied; every write and CREATE: denied after switching read-only off.
- `product_ro`: all `product` columns readable (exporter allow-list by construction); every write/TRUNCATE/CREATE denied after switching read-only off; `raw`, `pulso`, `public.pulso_jobs`: denied.
- `pulso_loader`: INSERT/DELETE on raw ok, UPDATE denied, `pulso` schema denied. (It can also write `product`, which the simulator relies on.)
- Observation: with read-only left at its default, writes fail with "read-only transaction" first; the grant layer is what holds when it is switched off, and it does.

## Failures found and fixed (each: RED commit, then GREEN commit, `Team: CL`)

1. **Simulator Postgres sink could not run as a writer role** (`permission denied for database`): `CREATE SCHEMA/TABLE IF NOT EXISTS` still needs CREATE privilege. Fix: `ensure_schema` probes `to_regclass` and issues DDL only for missing tables. Tests: `test_ensure_schema_issues_no_ddl_when_tables_exist`.
2. **`product.customer_case_slots` kept duplicate rows** (403 rows for 400 customers) when one batch touched a customer twice: delete-all then insert-all (the SQLite sink deletes per row). Fix: last row per customer wins within a batch. Verified live 400/400.
3. **`PgStore` panicked on its first successor/correlation/release write**: migration 0052's closed `pulso_ca_docs.ns` set lacks `engine_release`, `release_correlation`, `release_event`, `successor_run`, which the server writes (`correlation/mod.rs`). Seen as `put_doc_new_has_one_winner_under_8_threads` failing with a CHECK violation. Fix: additive migration `0054_pulso_ca_docs_release_ns.sql` (0052 byte-identical), applied by `PgStore::connect` after 0052; new test `every_namespace_the_server_writes_is_accepted_on_every_store`.
4. Test hygiene: the new `pg_live_sim` e2e would have failed under `PULSO_REQUIRE_POSTGRES=1` alone (needs a simulator-populated database); it now has its own `PULSO_REQUIRE_SIM_E2E`.

Noted, not fixed: `test_event_types_are_admitted_in_catalog_1_1_0` only passes from the repo root (relative path); `--overwrite` restarts `event_log.sequence` at 1, so a source whose watermark is already past that point would see nothing until its watermark row is reset (the simulator cannot append to an existing log).

## Reproduce

Gated e2e (`seams/crates/sources/tests/pg_live_sim.rs`): populate a db built from `db/sql` + `migrations/`, run the simulator as `pulso_loader`
(`PULSO_PRODUCT_SIM_PG_DSN`), set `PULSO_E2E_RO_DSN` (`pulso_product_ro`) and `PULSO_E2E_APP_DSN` (`pulso_app`), run
`cargo test -p sources --test pg_live_sim -- --test-threads=1`; the follow test also needs `PULSO_E2E_STOP_FILE` pointing at the
simulator's `--stop-file`.

## Proven live vs still unproven

Proven live: `db/sql` DDL and grants (+ idempotent re-apply), role least privilege, `load_parquet.py`, simulator Postgres sink (backfill, follow, overwrite, dense sequence), migrations 0001-0054, `pg` crate conformance/race/engine tests, `PgStore` (control-api), `PostgresProduct` adapter on the real schema, `PgStore` watermark with CAS, kill-before-commit resume, follow.
Unproven: `DatasetPg` against loader-populated `raw` data on the real DDL (only the hand-made schema in `pg_live.rs`); a true process kill (kill is simulated by a store that errors instead of committing, with fresh connections afterwards); multi-process watermark contention; control-api server binary over Postgres end to end (store layer only); loader with real (non-synthetic) Parquet volume.

## Teardown

`podman rm -f -v` removed the container and its anonymous volume; `podman ps -a` on `pulso-dev` afterwards lists only the two pre-existing `pulso-local-*` Created containers. Secret files deleted.
