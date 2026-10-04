-- Additive fix found by the first live Postgres run: the control-api writes four more document namespaces than the closed
-- set of 0052 allowed (`engine_release`, `release_correlation`, `release_event`, `successor_run`), so PgStore panicked on
-- the first successor/correlation/publish write. 0052 is left byte-identical (its checksum is already recorded).
ALTER TABLE pulso_ca_docs DROP CONSTRAINT IF EXISTS pulso_ca_docs_ns_check;
ALTER TABLE pulso_ca_docs ADD CONSTRAINT pulso_ca_docs_ns_check CHECK (ns IN (
    'ingest_ledger', 'ingest_cursor', 'ingest_receipt', 'ingest_quarantine', 'grant',
    'lab_session', 'lab_query', 'lab_result', 'lab_receipt', 'run_events', 'wiki',
    'engine_release', 'release_correlation', 'release_event', 'successor_run'));
