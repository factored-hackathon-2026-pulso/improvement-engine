"""Read-only platform sources: SQLite (guarded by an engine authorizer) and Postgres (read-only transaction).

All SQL is produced by `policy.select_sql`, so a denied table or column is rejected before any query text exists."""

from __future__ import annotations

import json
import sqlite3
from collections.abc import Iterator
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any

from .catalog import parse_ts
from .policy import ALLOWED_COLUMNS, assert_table_allowed, install_sqlite_guard, select_sql

REQUIRED_EVENT_COLUMNS = ("sequence", "event_id", "event_type", "event_time", "ingested_at", "payload")
CAPABILITY_TABLES = ("routing_step", "identity_check", "copilot_query", "tool_call", "approval", "suggestion",
                     "signal", "component", "case_close", "teams", "admin_roster")


class SchemaDrift(RuntimeError):
    code = "pulso:schema_drift"


@dataclass(frozen=True)
class RawEvent:
    sequence: int
    event_id: str
    event_type: str
    entity_id: str | None
    case_id: str | None
    actor_id: str | None
    event_time: datetime | None
    ingested_at: datetime | None
    payload: Any
    tenant_id: str | None
    problem: str | None = None  # bad_timestamp | payload_unparseable: such rows are quarantined, never forwarded


class SqlSource:
    """Shared logic; subclasses supply `_rows`, `table_columns` and `table_names`."""

    placeholder = "?"

    def _rows(self, sql: str, params: tuple[Any, ...] = ()) -> list[tuple[Any, ...]]:
        raise NotImplementedError

    def table_columns(self, table: str) -> list[str]:
        raise NotImplementedError

    def table_names(self) -> set[str]:
        raise NotImplementedError

    def close(self) -> None:
        pass

    # --- schema ---
    def _present(self, table: str, wanted: tuple[str, ...]) -> list[str]:
        have = set(self.table_columns(assert_table_allowed(table)))
        return [c for c in wanted if c in have]

    def event_columns(self) -> list[str]:
        have = set(self.table_columns("event_log"))
        missing = [c for c in REQUIRED_EVENT_COLUMNS if c not in have]
        if missing:
            raise SchemaDrift(f"event_log lacks required columns: {missing}")
        return [c for c in ALLOWED_COLUMNS["event_log"] if c in have]

    def unexpected_columns(self) -> dict[str, list[str]]:
        out: dict[str, list[str]] = {}
        for table in ALLOWED_COLUMNS:
            extra = sorted(set(self.table_columns(table)) - set(ALLOWED_COLUMNS[table]))
            if extra:
                out[table] = extra
        return out

    # --- events ---
    def _event(self, cols: list[str], row: tuple[Any, ...]) -> RawEvent:
        d = dict(zip(cols, row, strict=True))
        problem: str | None = None
        try:
            et, it = parse_ts(d["event_time"]), parse_ts(d["ingested_at"])
        except (ValueError, TypeError):
            et = it = None
            problem = "bad_timestamp"
        payload = d["payload"]
        if isinstance(payload, str | bytes):
            try:
                payload = json.loads(payload)
            except ValueError:
                payload, problem = None, problem or "payload_unparseable"
        actor = d.get("actor_id")
        return RawEvent(int(d["sequence"]), str(d["event_id"]), str(d["event_type"]), d.get("entity_id"),
                        d.get("case_id"), None if actor is None else str(actor), et, it, payload, d.get("tenant_id"),
                        problem)

    def events_after(self, sequence: int, limit: int) -> list[RawEvent]:
        cols = self.event_columns()
        sql = select_sql("event_log", cols, where=f"sequence > {self.placeholder}", order_by="sequence", limit=limit)
        return [self._event(cols, r) for r in self._rows(sql, (sequence,))]

    def events_between(self, lo: int, hi: int, limit: int) -> list[RawEvent]:
        cols = self.event_columns()
        sql = select_sql("event_log", cols, where=f"sequence BETWEEN {self.placeholder} AND {self.placeholder}",
                         order_by="sequence", limit=limit)
        return [self._event(cols, r) for r in self._rows(sql, (lo, hi))]

    def iter_events(self, page: int = 500) -> Iterator[RawEvent]:
        after = 0
        while True:
            rows = self.events_after(after, page)
            if not rows:
                return
            yield from rows
            after = rows[-1].sequence

    # --- dimensions (immutable columns only; never state, names or text) ---
    def case_customers(self) -> dict[str, str]:
        cols = self._present("cases", ("case_id", "customer_id"))
        if cols != ["case_id", "customer_id"]:
            return {}
        return {str(c): str(u) for c, u in self._rows(select_sql("cases", cols))}

    def simulator_customers(self) -> set[str]:
        cols = self._present("customers", ("customer_id", "simulator"))
        if cols != ["customer_id", "simulator"]:
            return set()
        return {str(c) for c, s in self._rows(select_sql("customers", cols)) if s in (1, True, "1", "true", "t")}

    def staff_dimension(self) -> list[dict[str, Any]]:
        cols = self._present("staff", ALLOWED_COLUMNS["staff"])
        return [dict(zip(cols, r, strict=True)) for r in self._rows(select_sql("staff", cols, order_by="staff_id"))]

    def turn_sequence_gaps(self) -> dict[str, list[int]]:
        """Per case, missing `turns.sequence` values inside 1..max (completeness proof of the append-only table)."""
        cols = self._present("turns", ("case_id", "sequence"))
        if cols != ["case_id", "sequence"]:
            return {}
        seqs: dict[str, set[int]] = {}
        for case_id, seq in self._rows(select_sql("turns", cols)):
            seqs.setdefault(str(case_id), set()).add(int(seq))
        return {c: sorted(set(range(1, max(s) + 1)) - s) for c, s in seqs.items()
                if set(range(1, max(s) + 1)) - s}


class SqliteSource(SqlSource):
    def __init__(self, path: str | Path) -> None:
        uri = Path(path).resolve().as_uri() + "?mode=ro"
        self._db = sqlite3.connect(uri, uri=True, isolation_level=None)
        install_sqlite_guard(self._db)

    def close(self) -> None:
        self._db.close()

    def _rows(self, sql: str, params: tuple[Any, ...] = ()) -> list[tuple[Any, ...]]:
        return list(self._db.execute(sql, params).fetchall())

    def table_columns(self, table: str) -> list[str]:
        t = assert_table_allowed(table)
        return [r[1] for r in self._db.execute(f"PRAGMA table_info({t})").fetchall()]

    def table_names(self) -> set[str]:
        return {r[0] for r in self._db.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()}


class PostgresSource(SqlSource):
    """Read-only Postgres reader. Use a role with SELECT only on the allow-listed columns (infra: PL-L5)."""

    placeholder = "%s"

    def __init__(self, dsn: str, schema: str = "public") -> None:
        import psycopg

        self._schema = schema
        self._conn = psycopg.connect(dsn, autocommit=True)
        self._conn.read_only = True

    def close(self) -> None:
        self._conn.close()

    def _rows(self, sql: str, params: tuple[Any, ...] = ()) -> list[tuple[Any, ...]]:
        with self._conn.cursor() as cur:
            cur.execute(sql, params)  # type: ignore[arg-type]
            return list(cur.fetchall())

    def table_columns(self, table: str) -> list[str]:
        t = assert_table_allowed(table)
        return [r[0] for r in self._rows(
            "SELECT column_name FROM information_schema.columns WHERE table_schema = %s AND table_name = %s "
            "ORDER BY ordinal_position", (self._schema, t))]

    def table_names(self) -> set[str]:
        return {r[0] for r in self._rows(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = %s", (self._schema,))}
