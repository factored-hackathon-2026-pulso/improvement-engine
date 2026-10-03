"""Access policy of the `platform_live` source (spec 32.2 items 1-3): allow-list by default, credential tables denied
by construction.

Every query is built from `select_sql`, which asserts table and columns BEFORE any SQL text exists. The SQLite
backend additionally installs an engine-level authorizer so even a hand-written statement cannot read a denied
table/column or write anything."""

from __future__ import annotations

import re
import sqlite3
from collections.abc import Callable

DENIED_TABLES = ("login_accounts", "mfa_challenges", "staff_sessions")

# table -> readable columns. Mutable `cases` state columns (status, close_reason, close_note, assigned_analyst_id,
# closed_at, version) are NOT readable: state comes from events (spec 32.2 item 3). Names, emails and message text
# are never readable either.
ALLOWED_COLUMNS: dict[str, tuple[str, ...]] = {
    "event_log": ("sequence", "event_id", "event_type", "entity_id", "case_id", "actor_id", "actor_type", "event_time",
                  "ingested_at", "payload", "tenant_id"),
    "cases": ("case_id", "customer_id", "channel", "language", "priority", "previous_case_id", "sla_due_at",
              "created_at", "tenant_id"),
    "customers": ("customer_id", "simulator"),
    "staff": ("staff_id", "roles", "languages", "team_id", "active"),
    "turns": ("turn_id", "case_id", "sequence", "author", "created_at"),
    "assignments": ("assignment_id", "case_id", "staff_id", "reason", "policy_rule_id", "strategy",
                    "open_cases_at_assignment", "waited_seconds", "previous_staff_id", "paused_override",
                    "created_at"),
    "customer_case_slots": ("customer_id", "open_case_id"),
}
ALLOWED_TABLES = tuple(ALLOWED_COLUMNS)

_IDENT = re.compile(r"^[a-z_][a-z0-9_]*$")


class AccessDenied(PermissionError):
    """Raised before any query when a table or column is outside the allow-list."""


def _norm(name: str) -> str:
    return name.strip().strip('"`[]').lower()


def assert_table_allowed(table: str) -> str:
    t = _norm(table)
    if t in DENIED_TABLES or t not in ALLOWED_COLUMNS or not _IDENT.match(t):
        raise AccessDenied(f"table not allowed: {t!r}")
    return t


def assert_columns_allowed(table: str, columns: list[str] | tuple[str, ...]) -> list[str]:
    t = assert_table_allowed(table)
    cols = [_norm(c) for c in columns]
    bad = [c for c in cols if c not in ALLOWED_COLUMNS[t]]
    if bad:
        raise AccessDenied(f"columns not allowed on {t}: {bad}")
    return cols


def select_sql(table: str, columns: list[str] | tuple[str, ...], *, where: str = "", order_by: str = "",
               limit: int | None = None) -> str:
    """Build a SELECT after the assertions. `where`/`order_by` may only use allowed identifiers and `?` / `%s` params."""
    t = assert_table_allowed(table)
    cols = assert_columns_allowed(t, columns)
    sql = f"SELECT {', '.join(cols)} FROM {t}"
    for clause, kw in ((where, "WHERE"), (order_by, "ORDER BY")):
        if clause:
            for ident in re.findall(r"[A-Za-z_][A-Za-z0-9_]*", re.sub(r"'[^']*'|%s|\?", "", clause)):
                if ident.upper() in {"AND", "OR", "IS", "NOT", "NULL", "IN", "ASC", "DESC", "BETWEEN"}:
                    continue
                if ident.lower() not in ALLOWED_COLUMNS[t]:
                    raise AccessDenied(f"identifier not allowed in {kw} on {t}: {ident}")
            sql += f" {kw} {clause}"
    if limit is not None:
        sql += f" LIMIT {int(limit)}"
    return sql


# --- SQLite engine-level guard -------------------------------------------------------------------------------------
_SQLITE_READ, _SQLITE_SELECT, _SQLITE_FUNCTION, _SQLITE_PRAGMA = 20, 21, 31, 19
_SQLITE_RECURSIVE = 33
_OK, _DENY = sqlite3.SQLITE_OK, sqlite3.SQLITE_DENY


def install_sqlite_guard(conn: sqlite3.Connection, on_read: Callable[[str, str], None] | None = None) -> None:
    """Deny every write and every read outside the allow-list at the engine (authorizer), not by convention."""

    def authorizer(action: int, a1: str | None, a2: str | None, _db: str | None, _src: str | None) -> int:
        if action in (_SQLITE_SELECT, _SQLITE_FUNCTION, _SQLITE_RECURSIVE):
            return _OK
        if action == _SQLITE_READ:
            table, column = (a1 or "").lower(), (a2 or "").lower()
            if on_read is not None:
                on_read(table, column)
            if table in ("sqlite_master", "sqlite_schema"):
                return _OK  # schema names only (drift detection); never row data of a table
            if table in DENIED_TABLES or table not in ALLOWED_COLUMNS:
                return _DENY
            if column and column not in ALLOWED_COLUMNS[table]:
                return _DENY
            return _OK
        if action == _SQLITE_PRAGMA:
            return _OK if (a1 or "").lower() == "table_info" and (a2 or "").lower() in ALLOWED_COLUMNS else _DENY
        return _DENY

    conn.set_authorizer(authorizer)
