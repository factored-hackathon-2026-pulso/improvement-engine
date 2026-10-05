# sources (R1M)

Read-only `SourceAdapter` port plus the monitor tick behind `pulso monitor` (design: `docs/plan-real/r1-real-system-design.md`
sections 2, 4, 5, 7).

- `policy`: allow-list/denylist, mirrored from `platform-exporter/.../policy.py` and drift-tested; hard cap 10 000 rows per read.
- Adapters: `sqlite` (`product-sqlite`: `SQLITE_OPEN_READ_ONLY`, `query_only`, engine authorizer), `pg_product` (`product-postgres`:
  session forced `default_transaction_read_only=on`, verified), `pg_dataset` (`dataset-pg`: `raw`/`augmented`, maps case, turn and
  close rows to `case.opened`, `turn.created`, `case.closed`; no E0 text column is selected).
- Watermarks: `store` (memory, file) and `pg_store` (`pulso.source_watermark`, migration `0053`). One row per `source_id`, bound to
  one adapter and one watermark kind, compare-and-set, never backwards. Product: `event_log.sequence`; datasets:
  `_ingested_at` + `_batch_id` + row key.
- `monitor::tick`: read batch, quarantine denied/unknown event types, write package `packages/<id>/{events.ndjson,manifest.json}`,
  run the engine sensor job (`FileStore` job, deterministic id), write `runs/<run_id>.json`, commit the watermark LAST
  (at-least-once, idempotent replay).

Run record fields: `data_mode`, `data_origin`, `source_id`, `adapter`, `data_class`, `watermark_from/to`, `label`, `evidence`
(`sensor: claude-standin`, `release`/`observation: simulated`, `model: none`), `history`, `signals` (history-free values;
history-dependent ones report `insufficient_history` with have/need/missing until 14 days and 200 cases).

Tests: `cargo test -j 1 -p sources` (offline). Live Postgres tests (`tests/pg_live.rs`) skip unless `PULSO_TEST_PG_ADMIN` is set
(admin DSN of a throw-away server; `PULSO_REQUIRE_POSTGRES=1` makes a missing server a failure).

Known gaps: the sensor stand-in runner does not read the package (its numbers are fixed; the real runner reads E0 Parquet);
`dataset-pg` does not yet derive `case.assigned`/`case.first_responded`/`case.queued`; `event_log.payload` is never read;
history-dependent signals beyond week-over-week are `not_implemented`; the Postgres adapters were not exercised against a live server.
