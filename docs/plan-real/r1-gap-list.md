# R1 gap list (for the user to route)

What the product team, the data team or the agent-core team must supply. Nothing here blocks unrelated work.

| ID | Owner | Gap | Needed by |
|---|---|---|---|
| G1 | Product team | Postgres-backed support-platform: today SQLite default, Postgres untested, no migrations, slice 12 vs our contract 1.1.0 (a492bfa). Until then the SQLite adapter (snapshot into `product.*` or read-only file) is the only real path. Also a contract refresh to the current slice. | R3 |
| G2 | Product team | EXT-2: release and observation events (publish, rollback, window metrics), and event types for escalations and calls (`case.escalated`, `call.*`) in the catalog. Until then release/observation stay `simulated` and escalations/calls have no allow-listed columns. | R3 |
| G3 | Data + Product | Enum reconciliation for `channel` and `priority` across E0, bank CSV and platform. | R1 mapping |
| G4 | Data team | Customer-id mapping: E0 pseudonym `PSN-` to bank `CLI-` to platform customer ids (the `pseudonym_map` is never loaded; mapping must be supplied as a keyed, non-reversible join table or by the canonical layer). | R1 augmented |
| G5 | agent-core team (via the user) | Identifiers of the agent and prompts the platform runs, the staging environment, and the publish mechanism, so proposals target real platform artifacts instead of the seeded world. | R3 |
| G6 | User | Upload E0 Parquet (9 tables) and the 13 bank CSVs (CSV loader path not built; only Parquet loader exists, convert or extend). | R1 dataset mode |
| G7 | User / L-PG | `pulso.source_watermark` and `pulso.run_source` migrations (L-PG range), role logins and passwords from secrets. | R1 |
| G8 | Data team | Canonical bank-case columns for `augmented.cases` (the dbt union of E0 and bank cases has more columns than the E0 shape in `db/catalog.py`). | R1 augmented |
| G9 | Product/Platform | Payload treatment guarantee: nothing with free text reaches `product.event_log.payload` (exporter treatment runs first). | R3 |
| G10 | Infra | RDS instance, one bucket with the prefixes in section 8, Secrets Manager entries, ECS task definition. | R4 |
