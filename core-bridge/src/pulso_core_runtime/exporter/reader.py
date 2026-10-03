"""Read-only Core reader: one connection per poll, REPEATABLE READ READ ONLY, exactly three tables.

Never uses AuditSink.read, PostgresOutbox.pending or mark_delivered; never touches `core_eval`; the only DSN is
the `exporter_ro` one (no Core DSN reaches Rust services)."""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from typing import Any

import psycopg
from psycopg import IsolationLevel

from .config import EVAL_MISCONFIGURED, SCHEMA_DRIFT, ExporterConfig

EXPECTED_COLUMNS: dict[str, set[str]] = {
    "audit_events": {"run_id", "seq", "event_id", "type", "release", "ts", "prev_hash", "hash", "event_json"},
    "reg_events": {"seq", "event_json"},
    "outbox": {"message_id", "seq", "message_json", "delivered_at"},
}


class ExporterError(RuntimeError):
    def __init__(self, code: str, detail: str = "") -> None:
        super().__init__(f"{code}: {detail}" if detail else code)
        self.code = code


@dataclass(frozen=True)
class AuditRow:
    run_id: str
    seq: int
    event_id: str
    type: str
    ts: Any
    prev_hash: str
    hash: str
    event_json: str  # exact stored text, never re-serialised


@dataclass(frozen=True)
class SeqRow:
    seq: int
    body: str


class Snapshot:
    def __init__(self, conn: psycopg.Connection[Any]) -> None:
        self._c = conn

    def heads(self) -> dict[str, int]:
        return {r[0]: r[1] for r in self._c.execute("SELECT run_id, max(seq) FROM audit_events GROUP BY run_id")}

    def audit_rows(self, run_id: str, after_seq: int, limit: int) -> list[AuditRow]:
        rows = self._c.execute(
            "SELECT run_id, seq, event_id, type, ts, prev_hash, hash, event_json FROM audit_events "
            "WHERE run_id=%s AND seq>%s ORDER BY seq LIMIT %s", (run_id, after_seq, limit)).fetchall()
        return [AuditRow(*r) for r in rows]

    def audit_prefix(self, run_id: str, through_seq: int) -> list[AuditRow]:
        rows = self._c.execute(
            "SELECT run_id, seq, event_id, type, ts, prev_hash, hash, event_json FROM audit_events "
            "WHERE run_id=%s AND seq<=%s ORDER BY seq", (run_id, through_seq)).fetchall()
        return [AuditRow(*r) for r in rows]

    def seq_rows(self, table: str, after: int, limit: int) -> list[SeqRow]:
        col = {"reg_events": "event_json", "outbox": "message_json"}[table]
        q = f"SELECT seq, {col} FROM {table} WHERE seq>%s ORDER BY seq LIMIT %s"
        return [SeqRow(*r) for r in self._c.execute(q, (after, limit)).fetchall()]  # type: ignore[arg-type]

    def seq_rows_in(self, table: str, seqs: list[int]) -> list[SeqRow]:
        if not seqs:
            return []
        col = {"reg_events": "event_json", "outbox": "message_json"}[table]
        q = f"SELECT seq, {col} FROM {table} WHERE seq = ANY(%s) ORDER BY seq"
        return [SeqRow(*r) for r in self._c.execute(q, (seqs,)).fetchall()]  # type: ignore[arg-type]


class CoreReader:
    def __init__(self, dsn: str, cfg: ExporterConfig) -> None:
        self._dsn, self._cfg = dsn, cfg

    def _guard(self, conn: psycopg.Connection[Any]) -> None:
        db, user, role = conn.execute(
            "SELECT current_database(), current_user, "
            "(SELECT rolsuper OR rolbypassrls OR rolcreatedb FROM pg_roles WHERE rolname=current_user)").fetchone()  # type: ignore[misc]
        cfg = self._cfg
        if db != cfg.expected_runtime_db or db == cfg.expected_eval_db:
            raise ExporterError(EVAL_MISCONFIGURED, f"connected to {db}")
        if role:
            raise ExporterError(EVAL_MISCONFIGURED, "exporter role is over-privileged")
        exists = conn.execute("SELECT 1 FROM pg_database WHERE datname=%s", (cfg.expected_eval_db,)).fetchone()
        if exists and conn.execute("SELECT has_database_privilege(current_user, %s, 'CONNECT')",
                                   (cfg.expected_eval_db,)).fetchone()[0]:  # type: ignore[index]
            raise ExporterError(EVAL_MISCONFIGURED, "exporter role can connect to the eval database")
        cols: dict[str, set[str]] = {}
        for t, c in conn.execute(
                "SELECT table_name, column_name FROM information_schema.columns WHERE table_schema='public' "
                "AND table_name = ANY(%s)", (list(EXPECTED_COLUMNS),)):
            cols.setdefault(t, set()).add(c)
        if cols != EXPECTED_COLUMNS:
            raise ExporterError(SCHEMA_DRIFT, "core tables differ from the pinned schema")

    @contextmanager
    def snapshot(self) -> Iterator[Snapshot]:
        conn = psycopg.connect(self._dsn, connect_timeout=5)
        try:
            conn.isolation_level = IsolationLevel.REPEATABLE_READ
            conn.read_only = True
            self._guard(conn)
            yield Snapshot(conn)
        finally:
            conn.rollback()
            conn.close()
