"""Receipt state machine over `pulso_bridge.receipts`: single-statement CAS, terminal states final.
Also the invocation-context rows and the budget meter (same bridge-owned schema)."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from decimal import Decimal, InvalidOperation
from typing import Any

import psycopg
from psycopg.rows import dict_row
from psycopg.types.json import Jsonb

STATES = ("prepared", "sent", "binding_confirmed", "terminal_ok", "terminal_failed", "unknown", "manual_reconcile")
TERMINAL = frozenset({"terminal_ok", "terminal_failed"})
NON_TERMINAL = frozenset(STATES) - TERMINAL

# target -> allowed source states. Terminal states have no outgoing edge.
ALLOWED_FROM: dict[str, frozenset[str]] = {
    "sent": frozenset({"prepared"}),
    "binding_confirmed": frozenset({"sent"}),
    "terminal_ok": frozenset({"sent", "binding_confirmed", "unknown", "manual_reconcile"}),
    "terminal_failed": frozenset({"prepared", "sent", "binding_confirmed", "unknown", "manual_reconcile"}),
    "unknown": frozenset({"sent", "binding_confirmed", "unknown"}),
    "manual_reconcile": frozenset({"prepared", "sent", "binding_confirmed", "unknown", "manual_reconcile"}),
}


@dataclass(frozen=True)
class Receipt:
    tenant_id: str
    idempotency_key: str
    request_digest: str
    stage: str
    job_id: str
    attempt: int
    release_id: str
    task_binding_ref: str
    principal_id: str
    state: str
    version: int
    core_run_id: str | None = None
    outcome: str | None = None
    reason: str | None = None
    receipt: dict[str, Any] | None = None
    updated_at: datetime | None = None

    @property
    def terminal(self) -> bool:
        return self.state in TERMINAL


def _row(r: dict[str, Any]) -> Receipt:
    return Receipt(**{k: r[k] for k in Receipt.__dataclass_fields__})


class ReceiptStore:
    def __init__(self, dsn: str) -> None:
        self._dsn = dsn

    def _conn(self) -> psycopg.Connection[dict[str, Any]]:
        return psycopg.connect(self._dsn, row_factory=dict_row)  # type: ignore[return-value]

    def begin(self, *, tenant_id: str, key: str, digest: str, stage: str, job_id: str, attempt: int,
              release_id: str, task_binding_ref: str, principal_id: str) -> tuple[Receipt, bool]:
        """Single-flight insert. Returns (row, created). The advisory lock only serialises insert+read."""
        with self._conn() as conn:
            conn.execute("SELECT pg_advisory_xact_lock(hashtextextended(%s, 0))", (f"{tenant_id}|{key}",))
            cur = conn.execute(
                "INSERT INTO pulso_bridge.receipts (tenant_id, idempotency_key, request_digest, stage, job_id,"
                " attempt, release_id, task_binding_ref, principal_id, state) VALUES"
                " (%s,%s,%s,%s,%s,%s,%s,%s,%s,'prepared') ON CONFLICT DO NOTHING RETURNING *",
                (tenant_id, key, digest, stage, job_id, attempt, release_id, task_binding_ref, principal_id))
            row = cur.fetchone()
            if row is not None:
                return _row(row), True
            row = conn.execute("SELECT * FROM pulso_bridge.receipts WHERE tenant_id=%s AND idempotency_key=%s",
                               (tenant_id, key)).fetchone()
        assert row is not None
        return _row(row), False

    def get(self, tenant_id: str, key: str) -> Receipt | None:
        with self._conn() as conn:
            row = conn.execute("SELECT * FROM pulso_bridge.receipts WHERE tenant_id=%s AND idempotency_key=%s",
                               (tenant_id, key)).fetchone()
        return _row(row) if row else None

    def get_by_run(self, tenant_id: str, core_run_id: str) -> Receipt | None:
        with self._conn() as conn:
            row = conn.execute("SELECT * FROM pulso_bridge.receipts WHERE tenant_id=%s AND core_run_id=%s",
                               (tenant_id, core_run_id)).fetchone()
        return _row(row) if row else None

    def get_by_binding_ref(self, task_binding_ref: str) -> Receipt | None:
        with self._conn() as conn:
            row = conn.execute("SELECT * FROM pulso_bridge.receipts WHERE task_binding_ref=%s",
                               (task_binding_ref,)).fetchone()
        return _row(row) if row else None

    def transition(self, tenant_id: str, key: str, to: str, *, core_run_id: str | None = None,
                   outcome: str | None = None, reason: str | None = None,
                   receipt: dict[str, Any] | None = None) -> Receipt | None:
        """CAS in one UPDATE; None when the row is not in an allowed source state (lost race / terminal)."""
        sources = ALLOWED_FROM[to]
        with self._conn() as conn:
            row = conn.execute(
                "UPDATE pulso_bridge.receipts SET state=%s, version=version+1, updated_at=now(),"
                " core_run_id=COALESCE(%s, core_run_id), outcome=COALESCE(%s, outcome),"
                " reason=COALESCE(%s, reason), receipt=COALESCE(%s, receipt)"
                " WHERE tenant_id=%s AND idempotency_key=%s AND state = ANY(%s) RETURNING *",
                (to, core_run_id, outcome, reason, Jsonb(receipt) if receipt is not None else None,
                 tenant_id, key, sorted(sources))).fetchone()
        return _row(row) if row else None

    def discard_prepared(self, tenant_id: str, key: str) -> bool:
        """Drop a row that never left `prepared` (nothing was sent), so a retry with the same key starts clean."""
        with self._conn() as conn:
            cur = conn.execute("DELETE FROM pulso_bridge.receipts WHERE tenant_id=%s AND idempotency_key=%s"
                               " AND state='prepared'", (tenant_id, key))
            return cur.rowcount == 1

    # --- invocation contexts and budget meter -------------------------------------------------------

    def save_context(self, ref: str, tenant_id: str, job_id: str, key: str, context: dict[str, Any],
                     expires_at: Any) -> None:
        with self._conn() as conn:
            conn.execute(
                "INSERT INTO pulso_bridge.invocation_contexts (task_binding_ref, tenant_id, job_id,"
                " idempotency_key, context, expires_at) VALUES (%s,%s,%s,%s,%s,%s) ON CONFLICT DO NOTHING",
                (ref, tenant_id, job_id, key, Jsonb(context), expires_at))

    def delete_context(self, ref: str) -> None:
        with self._conn() as conn:
            conn.execute("UPDATE pulso_bridge.invocation_contexts SET deleted_at=now() "
                         "WHERE task_binding_ref=%s AND deleted_at IS NULL", (ref,))

    def context_row(self, ref: str) -> dict[str, Any] | None:
        with self._conn() as conn:
            return conn.execute("SELECT * FROM pulso_bridge.invocation_contexts WHERE task_binding_ref=%s",
                                (ref,)).fetchone()

    def meter_add(self, tenant_id: str, job_id: str, stage: str, attempt: int, *, calls: int = 0, tokens: int = 0,
                  cost_usd: str = "0", usage_known: bool = True) -> None:
        with self._conn() as conn:
            conn.execute(
                "INSERT INTO pulso_bridge.budget_meter (tenant_id, job_id, stage, attempt, calls, tokens,"
                " cost_usd, usage_known) VALUES (%s,%s,%s,%s,%s,%s,%s,%s) ON CONFLICT (tenant_id, job_id, stage,"
                " attempt) DO UPDATE SET calls=budget_meter.calls+EXCLUDED.calls,"
                " tokens=budget_meter.tokens+EXCLUDED.tokens, cost_usd=budget_meter.cost_usd+EXCLUDED.cost_usd,"
                " usage_known=budget_meter.usage_known AND EXCLUDED.usage_known, updated_at=now()",
                (tenant_id, job_id, stage, attempt, calls, tokens, cost_usd, usage_known))

    def meter_spend(self, tenant_id: str, job_id: str, stage: str, attempt: int, *, cost_usd: str, cap_usd: str,
                    calls: int = 1, tokens: int = 0) -> bool:
        """Atomic capped spend: one statement, applied only if the new total stays <= cap. Concurrent spenders
        serialise on the row lock and re-check the cap, so the total can never exceed it. A negative, non-finite or
        unparsable amount is refused (it would refund the budget)."""
        try:
            amount = Decimal(cost_usd)
        except (InvalidOperation, ValueError):
            return False
        if not amount.is_finite() or amount < 0:
            return False
        with self._conn() as conn:
            row = conn.execute(
                "INSERT INTO pulso_bridge.budget_meter (tenant_id, job_id, stage, attempt, calls, tokens, cost_usd)"
                " SELECT %s,%s,%s,%s,%s,%s,%s::numeric WHERE %s::numeric <= %s::numeric"
                " ON CONFLICT (tenant_id, job_id, stage, attempt) DO UPDATE SET calls=budget_meter.calls+EXCLUDED.calls,"
                " tokens=budget_meter.tokens+EXCLUDED.tokens, cost_usd=budget_meter.cost_usd+EXCLUDED.cost_usd,"
                " updated_at=now() WHERE budget_meter.cost_usd+EXCLUDED.cost_usd <= %s::numeric RETURNING 1",
                (tenant_id, job_id, stage, attempt, calls, tokens, cost_usd, cost_usd, cap_usd, cap_usd)).fetchone()
        return row is not None

    def meter_get(self, tenant_id: str, job_id: str, stage: str, attempt: int) -> dict[str, Any] | None:
        with self._conn() as conn:
            return conn.execute(
                "SELECT * FROM pulso_bridge.budget_meter WHERE tenant_id=%s AND job_id=%s AND stage=%s"
                " AND attempt=%s", (tenant_id, job_id, stage, attempt)).fetchone()

    # --- metering v2: atomic pre-reservation, settlement and the model-call ledger ------------------------------

    def meter_reserve(self, tenant_id: str, job_id: str, stage: str, attempt: int, *, amount: str,
                      cap_usd: str) -> bool:
        """Atomically hold `amount` against the cap: one statement, applied only if `spent + reserved + amount <= cap`
        (concurrent callers serialise on the row lock and re-check). Unparsable, negative or non-finite is refused."""
        try:
            hold = Decimal(amount)
        except (InvalidOperation, ValueError):
            return False
        if not hold.is_finite() or hold < 0:
            return False
        with self._conn() as conn:
            row = conn.execute(
                "INSERT INTO pulso_bridge.budget_meter (tenant_id, job_id, stage, attempt, reserved_usd)"
                " SELECT %s,%s,%s,%s,%s::numeric WHERE %s::numeric <= %s::numeric"
                " ON CONFLICT (tenant_id, job_id, stage, attempt) DO UPDATE SET"
                " reserved_usd=budget_meter.reserved_usd+EXCLUDED.reserved_usd, updated_at=now()"
                " WHERE budget_meter.cost_usd+budget_meter.reserved_usd+EXCLUDED.reserved_usd <= %s::numeric"
                " RETURNING 1", (tenant_id, job_id, stage, attempt, amount, amount, cap_usd, cap_usd)).fetchone()
        return row is not None

    def model_call_settle(self, tenant_id: str, job_id: str, stage: str, attempt: int, *, cap_usd: str,
                          release_usd: Decimal, cost_usd: Decimal, tokens: int, usage_known: bool, tokens_in: int,
                          tokens_out: int, outcome: str, binding_ref: str, **entry: Any) -> tuple[Decimal, bool]:
        """Always applies the spend (never dropped), releases `release_usd` of the reservation (0 keeps it as unknown)
        and appends the ledger row, in one transaction. Returns (total spent, over the cap)."""
        with self._conn() as conn:
            row = conn.execute(
                "INSERT INTO pulso_bridge.budget_meter (tenant_id, job_id, stage, attempt, calls, tokens, cost_usd,"
                " usage_known) VALUES (%s,%s,%s,%s,1,%s,%s::numeric,%s)"
                " ON CONFLICT (tenant_id, job_id, stage, attempt) DO UPDATE SET calls=budget_meter.calls+1,"
                " tokens=budget_meter.tokens+EXCLUDED.tokens, cost_usd=budget_meter.cost_usd+EXCLUDED.cost_usd,"
                " reserved_usd=GREATEST(budget_meter.reserved_usd-%s::numeric, 0),"
                " usage_known=budget_meter.usage_known AND EXCLUDED.usage_known, updated_at=now()"
                " RETURNING cost_usd", (tenant_id, job_id, stage, attempt, tokens, str(cost_usd), usage_known,
                                        str(release_usd))).fetchone()
            assert row is not None
            total = Decimal(row["cost_usd"])
            over = total > Decimal(cap_usd)
            self._ledger_insert(conn, tenant_id, job_id, stage, attempt,
                                "over_cap" if over and outcome == "ok" else outcome, binding_ref=binding_ref,
                                tokens_in=tokens_in, tokens_out=tokens_out, cost_usd=cost_usd,
                                usage_known=usage_known, over_cap=over, **entry)
        return total, over

    def ledger_record(self, tenant_id: str, job_id: str, stage: str, attempt: int, *, outcome: str,
                      binding_ref: str, **entry: Any) -> None:
        """One ledger row for a call that never reached the model (policy denied, budget exhausted) or whose spend is
        unknown (non-gateway error). No meter change."""
        with self._conn() as conn:
            self._ledger_insert(conn, tenant_id, job_id, stage, attempt, outcome, binding_ref=binding_ref, **entry)

    @staticmethod
    def _ledger_insert(conn: Any, tenant_id: str, job_id: str, stage: str, attempt: int, outcome: str, *,
                       binding_ref: str, tokens_in: int = 0, tokens_out: int = 0, cost_usd: Decimal = Decimal(0),
                       gateway_cost_usd: Decimal | None = None, usage_known: bool = True,
                       reserved_usd: Decimal = Decimal(0), over_cap: bool = False, price_mismatch: bool = False,
                       endpoint_alias: str | None = None, model_requested: str | None = None,
                       model_reported: str | None = None, reason: str | None = None) -> None:
        conn.execute(
            "INSERT INTO pulso_bridge.model_call_ledger (tenant_id, job_id, stage, attempt, binding_ref, outcome,"
            " reason, tokens_in, tokens_out, cost_usd, gateway_cost_usd, usage_known, reserved_usd, over_cap,"
            " price_mismatch, endpoint_alias, model_requested, model_reported)"
            " VALUES (%s,%s,%s,%s,%s,%s,%s,%s,%s,%s::numeric,%s::numeric,%s,%s::numeric,%s,%s,%s,%s,%s)",
            (tenant_id, job_id, stage, attempt, binding_ref, outcome, reason, tokens_in, tokens_out, str(cost_usd),
             None if gateway_cost_usd is None else str(gateway_cost_usd), usage_known, str(reserved_usd), over_cap,
             price_mismatch, endpoint_alias, model_requested, model_reported))

    def ledger_rows(self, tenant_id: str, job_id: str, stage: str, attempt: int) -> list[dict[str, Any]]:
        with self._conn() as conn:
            return conn.execute(
                "SELECT * FROM pulso_bridge.model_call_ledger WHERE tenant_id=%s AND job_id=%s AND stage=%s"
                " AND attempt=%s ORDER BY id", (tenant_id, job_id, stage, attempt)).fetchall()

    def meter_reconcile(self, tenant_id: str, job_id: str, stage: str, attempt: int) -> bool:
        with self._conn() as conn:
            cur = conn.execute(
                "UPDATE pulso_bridge.budget_meter SET reconciled=true, updated_at=now() WHERE tenant_id=%s AND"
                " job_id=%s AND stage=%s AND attempt=%s", (tenant_id, job_id, stage, attempt))
            return cur.rowcount == 1
