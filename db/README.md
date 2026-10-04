# db - one Postgres, schemas raw / augmented / product / pulso

DDL and loader for the shared database (design: `docs/plan-real/r1-real-system-design.md`).

- `catalog.py`: single source of truth (names and types only). `gen_ddl.py` renders `sql/*.sql`; never edit the SQL by hand
  (`python db/gen_ddl.py`, a test fails on drift).
- `sql/001_roles_schemas.sql`: schemas, NOLOGIN roles (`pulso_raw_ro`, `pulso_augmented_ro`, `pulso_product_ro`,
  `pulso_loader`, `pulso_app`), `default_transaction_read_only=on` on every reader. Passwords/LOGIN come from bootstrap secrets.
- `sql/010_raw.sql`: 9 E0 tables (`e0_*`) and 13 bank tables (`bank_*`), lossless. `labels`, `timeline`, `pseudonym_map` never exist.
- `sql/020_augmented.sql`: canonical layer. `sql/030_product.sql`: support-platform replica, exporter allow-list columns only
  (`event_log.sequence` is the watermark). Credential tables absent.
- `sql/090_grants.sql`: column-level SELECT for readers (free text, names, contact data not granted); loader gets SELECT/INSERT/DELETE.
- Every table has lineage columns `_batch_id`, `_source_file`, `_ingested_at`.

Apply in file order as a superuser/owner, then set role logins from secrets.

Loader: `PULSO_PG_LOADER_DSN=... python db/loader/load_parquet.py --schema raw --table e0_case --file case.parquet`
(needs `psycopg` and `pyarrow`). One transaction per file: delete the `_batch_id`, then COPY; re-running is idempotent
(default batch id = content hash). Refuses evaluator/unknown tables and unknown columns; prints counts only.

Tests (offline, no database): `cd db && uv run --python 3.12 --with pytest python -m pytest tests`.
A live check against a local container was not run in this slice.
