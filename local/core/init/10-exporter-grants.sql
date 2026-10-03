-- Applied by core-grants after core-migrate. exporter_ro: SELECT on exactly three tables (exporter/reader.py
-- READ_ONLY_TABLES), nothing on FORBIDDEN_TABLES, no CONNECT on core_eval. Idempotent.
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM exporter_ro;
GRANT SELECT ON audit_events, reg_events, outbox TO exporter_ro;
-- The runtime runs its own bridge migrations (schema pulso_bridge) as core_app.
GRANT CREATE ON DATABASE core_runtime TO core_app;
