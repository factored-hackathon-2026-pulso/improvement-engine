"""Receiver-owned replay tables (SQLite): `(iss, jti)` of service tokens and one-use request nonces.

Rows are retained through `exp + skew` with the same boundary the verifier uses, so a token cannot be replayed
while it is still acceptable. File-backed by default in compose (durable across restarts)."""

from __future__ import annotations

import sqlite3
import threading

DDL = (
    "CREATE TABLE IF NOT EXISTS jti_seen (iss TEXT NOT NULL, jti TEXT NOT NULL, retain_until INTEGER NOT NULL, "
    "PRIMARY KEY (iss, jti))",
    "CREATE TABLE IF NOT EXISTS nonce_seen (purpose TEXT NOT NULL, tenant_id TEXT NOT NULL, nonce TEXT NOT NULL, "
    "retain_until INTEGER NOT NULL, PRIMARY KEY (purpose, tenant_id, nonce))",
)


class ReplayStore:
    def __init__(self, path: str = ":memory:") -> None:
        self._db = sqlite3.connect(path, check_same_thread=False, isolation_level=None)
        self._lock = threading.Lock()
        with self._lock:
            for stmt in DDL:
                self._db.execute(stmt)

    def _consume(self, table: str, key: tuple[str, ...], retain_until: int, now: int) -> bool:
        cols = {"jti_seen": "iss, jti", "nonce_seen": "purpose, tenant_id, nonce"}[table]
        marks = ", ".join("?" for _ in key)
        with self._lock:
            self._db.execute(f"DELETE FROM {table} WHERE retain_until <= ?", (now,))  # noqa: S608
            cur = self._db.execute(  # noqa: S608
                f"INSERT OR IGNORE INTO {table} ({cols}, retain_until) VALUES ({marks}, ?)", (*key, retain_until)
            )
            return cur.rowcount == 1

    def consume_jti(self, iss: str, jti: str, *, retain_until: int, now: int) -> bool:
        """Atomically record `(iss, jti)`; False when already seen."""
        return self._consume("jti_seen", (iss, jti), retain_until, now)

    def consume_nonce(self, purpose: str, tenant_id: str, nonce: str, *, retain_until: int, now: int) -> bool:
        return self._consume("nonce_seen", (purpose, tenant_id, nonce), retain_until, now)
