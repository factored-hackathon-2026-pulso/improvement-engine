-- Applied by core-grants after core-migrate (core_runtime). `agentcore migrate --app-role core_app` grants only
-- audit_events and the registry tables; the engine tables stay owner-only, so the first POST /v1/runs would die with
-- InsufficientPrivilege. Least privilege: DML on exactly the engine tables + sequences; no DDL, nothing on other
-- roles. GRANTs are idempotent. exporter_ro is untouched (10-exporter-grants.sql: SELECT on three tables only).
GRANT SELECT, INSERT, UPDATE, DELETE ON runs, run_idempotency, turn_leases, turn_results, usage, handoffs, outbox TO core_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO core_app;
