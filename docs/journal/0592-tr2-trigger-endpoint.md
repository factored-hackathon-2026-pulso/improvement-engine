# 0592 TR2 trigger endpoint (UTC 2026-10-04T12:00Z, CLAUDE)

Lane TR2, branch `claude/tr2-trigger-endpoint` (from `claude/w8-consolidated`, PR 99 unmerged), owner L-CAPI.

Delivered
- `POST /internal/v1/automation/triggers` in debug-api: `pulso.trigger.v1`, bearer + CSRF, `Idempotency-Key` = `trigger_key`, replay returns the stored result, keyed job `trigger:<key>` through the new `TriggerAdmitter` trait (contract of `JobRepository::admit_keyed`), audit event `automation_trigger_received`, `GET /triggers` and `triggers` in the case-types projection.
- Free text and non-id subject fields are dropped, never copied to events. Doc: `seams/crates/debug-api/TRIGGERS.md`. OWNERS: `docs/journal/0592-*` to L-CAPI.

Tests: `cargo test -j 1 -p debug-api` (13 new in `tests/triggers.rs`: auth, CSRF, replay, conflict, bad payload, unknown kind, tenant, leakage, admission failure, projection).

Open: `pulso` does not yet inject its `JobRepository` (not serving automation), the runner ignores `trigger:*` jobs, and the poller sink sends no CSRF token.
