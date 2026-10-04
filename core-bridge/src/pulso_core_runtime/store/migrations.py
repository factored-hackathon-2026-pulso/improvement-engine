"""L3 bridge-owned tables in `pulso_bridge` (idempotent, applied after L2's `ensure_schema`).

Migrations are tracked in `pulso_bridge.migrations` (own table) so L2's `schema_version` check is untouched."""

from __future__ import annotations

import psycopg

MIGRATIONS: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("l3_001_receipts_contexts_budget", (
        "CREATE TABLE IF NOT EXISTS pulso_bridge.receipts ("
        " tenant_id text NOT NULL, idempotency_key text NOT NULL, request_digest text NOT NULL,"
        " stage text NOT NULL, job_id text NOT NULL, attempt integer NOT NULL, release_id text NOT NULL,"
        " task_binding_ref text NOT NULL, principal_id text NOT NULL, core_run_id text,"
        " state text NOT NULL CHECK (state IN ('prepared','sent','binding_confirmed','terminal_ok',"
        "   'terminal_failed','unknown','manual_reconcile')),"
        " outcome text, reason text, receipt jsonb, version integer NOT NULL DEFAULT 0,"
        " created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now(),"
        " PRIMARY KEY (tenant_id, idempotency_key))",
        "CREATE INDEX IF NOT EXISTS receipts_run ON pulso_bridge.receipts (core_run_id)",
        "CREATE TABLE IF NOT EXISTS pulso_bridge.invocation_contexts ("
        " task_binding_ref text PRIMARY KEY, tenant_id text NOT NULL, job_id text NOT NULL,"
        " idempotency_key text NOT NULL, context jsonb NOT NULL, expires_at timestamptz NOT NULL,"
        " deleted_at timestamptz)",
        "CREATE TABLE IF NOT EXISTS pulso_bridge.budget_meter ("
        " tenant_id text NOT NULL, job_id text NOT NULL, stage text NOT NULL, attempt integer NOT NULL,"
        " calls integer NOT NULL DEFAULT 0, tokens bigint NOT NULL DEFAULT 0,"
        " cost_usd numeric(20,8) NOT NULL DEFAULT 0, usage_known boolean NOT NULL DEFAULT true,"
        " reconciled boolean NOT NULL DEFAULT false, updated_at timestamptz NOT NULL DEFAULT now(),"
        " PRIMARY KEY (tenant_id, job_id, stage, attempt))",
    )),
    ("l3_002_model_call_ledger", (
        # Pre-reservation (spend held until a known outcome) next to the existing meter.
        "ALTER TABLE pulso_bridge.budget_meter ADD COLUMN IF NOT EXISTS reserved_usd numeric(20,8) NOT NULL DEFAULT 0",
        # Append-only record of EVERY model call outcome. Metadata only: no prompt, input, output or provider text.
        ("CREATE TABLE IF NOT EXISTS pulso_bridge.model_call_ledger ("
        " id bigserial PRIMARY KEY, tenant_id text NOT NULL, job_id text NOT NULL, stage text NOT NULL,"
        " attempt integer NOT NULL, binding_ref text NOT NULL,"
        " outcome text NOT NULL CHECK (outcome IN ('ok','timeout','unavailable','rate_limited','invalid_output',"
        "   'refused','over_cap','budget_exhausted','policy_denied','error')),"
        " reason text, tokens_in integer NOT NULL DEFAULT 0, tokens_out integer NOT NULL DEFAULT 0,"
        " cost_usd numeric(20,8) NOT NULL DEFAULT 0, gateway_cost_usd numeric(20,8),"
        " usage_known boolean NOT NULL DEFAULT true, reserved_usd numeric(20,8) NOT NULL DEFAULT 0,"
        " over_cap boolean NOT NULL DEFAULT false, price_mismatch boolean NOT NULL DEFAULT false,"
        " endpoint_alias text, model_requested text, model_reported text,"
        " created_at timestamptz NOT NULL DEFAULT now())"),
        ("CREATE INDEX IF NOT EXISTS model_call_ledger_scope ON pulso_bridge.model_call_ledger"
         " (tenant_id, job_id, stage, attempt, id)"),
    )),
    ("l3_003_ledger_guard_outcomes", (
        # The spend guard refuses before reserving: those refusals are ledgered too.
        "ALTER TABLE pulso_bridge.model_call_ledger DROP CONSTRAINT IF EXISTS model_call_ledger_outcome_check",
        ("ALTER TABLE pulso_bridge.model_call_ledger ADD CONSTRAINT model_call_ledger_outcome_check CHECK (outcome IN"
         " ('ok','timeout','unavailable','rate_limited','invalid_output','refused','over_cap','budget_exhausted',"
         "'policy_denied','error','kill_switch','ceiling_exceeded'))"),
    )),
)

BOOKKEEPING = ("CREATE TABLE IF NOT EXISTS pulso_bridge.migrations (name text PRIMARY KEY, "
               "applied_at timestamptz NOT NULL DEFAULT now())")


def apply_l3(dsn: str) -> None:
    with psycopg.connect(dsn, autocommit=True) as conn:
        conn.execute("CREATE SCHEMA IF NOT EXISTS pulso_bridge")
        conn.execute(BOOKKEEPING)  # type: ignore[arg-type]
        conn.execute("SELECT pg_advisory_lock(7301)")
        try:
            for name, stmts in MIGRATIONS:
                if conn.execute("SELECT 1 FROM pulso_bridge.migrations WHERE name=%s", (name,)).fetchone():
                    continue
                for stmt in stmts:
                    conn.execute(stmt)  # type: ignore[arg-type]
                conn.execute("INSERT INTO pulso_bridge.migrations (name) VALUES (%s) ON CONFLICT DO NOTHING",
                             (name,))
        finally:
            conn.execute("SELECT pg_advisory_unlock(7301)")


def l3_ready(dsn: str, timeout_s: int = 3) -> bool:
    try:
        with psycopg.connect(dsn, autocommit=True, connect_timeout=timeout_s) as conn:
            have = {r[0] for r in conn.execute("SELECT name FROM pulso_bridge.migrations").fetchall()}
        return {n for n, _ in MIGRATIONS} <= have
    except psycopg.Error:
        return False
