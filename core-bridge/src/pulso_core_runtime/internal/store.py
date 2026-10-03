"""`pulso_bridge` schema bootstrap (idempotent) and the Postgres-backed `jti_seen` replay table."""

from __future__ import annotations

from datetime import datetime
from typing import Any

import psycopg

SCHEMA_VERSION = 1
DDL = (
    "CREATE SCHEMA IF NOT EXISTS pulso_bridge",
    "CREATE TABLE IF NOT EXISTS pulso_bridge.schema_version (version integer PRIMARY KEY, "
    "applied_at timestamptz NOT NULL DEFAULT now())",
    "CREATE TABLE IF NOT EXISTS pulso_bridge.jti_seen (iss text NOT NULL, jti text NOT NULL, "
    "exp timestamptz NOT NULL, PRIMARY KEY (iss, jti))",
    "CREATE INDEX IF NOT EXISTS jti_seen_exp ON pulso_bridge.jti_seen (exp)",
)


def ensure_schema(dsn: str) -> None:
    with psycopg.connect(dsn, autocommit=True) as conn:
        for stmt in DDL:
            conn.execute(stmt)  # type: ignore[arg-type]
        conn.execute("INSERT INTO pulso_bridge.schema_version (version) VALUES (%s) ON CONFLICT DO NOTHING",
                     (SCHEMA_VERSION,))


def schema_ready(dsn: str, timeout_s: int = 3) -> bool:
    """Readiness: the bridge schema exists at the expected version. Fails closed, no detail."""
    try:
        with psycopg.connect(dsn, autocommit=True, connect_timeout=timeout_s) as conn:
            row = conn.execute("SELECT max(version) FROM pulso_bridge.schema_version").fetchone()
        return row is not None and row[0] == SCHEMA_VERSION
    except psycopg.Error:
        return False


class PgJtiStore:
    def __init__(self, dsn: str) -> None:
        self._dsn = dsn

    def consume(self, iss: str, jti: str, exp: datetime) -> bool:
        with psycopg.connect(self._dsn, autocommit=True) as conn:
            conn.execute("DELETE FROM pulso_bridge.jti_seen WHERE exp < now()")
            cur: Any = conn.execute(
                "INSERT INTO pulso_bridge.jti_seen (iss, jti, exp) VALUES (%s, %s, %s) ON CONFLICT DO NOTHING",
                (iss, jti, exp))
            return bool(cur.rowcount == 1)
