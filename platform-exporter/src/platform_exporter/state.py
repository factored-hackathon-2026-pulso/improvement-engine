"""Exporter local state: SQLite (WAL, synchronous=FULL) on its own volume, never inside the platform database.

Tables: partition_cursor, hole_ranges (inclusive; skipped=1 means a gap_suspected backfill request is open),
pending_batch (single row, stored before the POST), quarantine, counters, meta. `commit_ack` applies a batch delta, the
cursor and the pending-batch delete in ONE transaction."""

from __future__ import annotations

import json
import sqlite3
from dataclasses import dataclass
from pathlib import Path
from typing import Any

SCHEMA = """
CREATE TABLE IF NOT EXISTS partition_cursor(partition TEXT PRIMARY KEY, cursor TEXT, revision INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS hole_ranges(partition TEXT NOT NULL, lo INTEGER NOT NULL, hi INTEGER NOT NULL,
  first_seen REAL NOT NULL, skipped INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(partition, lo));
CREATE TABLE IF NOT EXISTS partition_status(partition TEXT PRIMARY KEY, status TEXT NOT NULL, reason TEXT);
CREATE TABLE IF NOT EXISTS pending_batch(id INTEGER PRIMARY KEY CHECK(id=1), partition TEXT NOT NULL,
  scan_mode TEXT NOT NULL, idem_key TEXT NOT NULL, body BLOB NOT NULL, delta TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS quarantine(event_id TEXT PRIMARY KEY, sequence INTEGER, event_type TEXT, reason TEXT);
CREATE TABLE IF NOT EXISTS batch_quarantine(idem_key TEXT PRIMARY KEY, partition TEXT, reason TEXT, body BLOB);
CREATE TABLE IF NOT EXISTS counters(k TEXT PRIMARY KEY, n INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS meta(k TEXT PRIMARY KEY, v TEXT NOT NULL);
"""


@dataclass(frozen=True)
class Pending:
    partition: str
    scan_mode: str
    idem_key: str
    body: bytes
    delta: dict[str, Any]


class ExporterState:
    def __init__(self, path: Path) -> None:
        self._db = sqlite3.connect(str(path), isolation_level=None)
        self._db.execute("PRAGMA journal_mode=WAL")
        self._db.execute("PRAGMA synchronous=FULL")
        self._db.executescript(SCHEMA)

    def close(self) -> None:
        self._db.close()

    # --- cursor ---
    def cursor(self, partition: str) -> tuple[str | None, int] | None:
        r = self._db.execute("SELECT cursor, revision FROM partition_cursor WHERE partition=?", (partition,)).fetchone()
        return (r[0], r[1]) if r else None

    def set_cursor(self, partition: str, cursor: str | None, revision: int) -> None:
        self._db.execute("INSERT INTO partition_cursor VALUES(?,?,?) ON CONFLICT(partition) DO UPDATE SET "
                         "cursor=excluded.cursor, revision=excluded.revision", (partition, cursor, revision))

    # --- holes / backfill requests ---
    def note_hole_range(self, partition: str, lo: int, hi: int, now: float) -> float:
        self._db.execute("INSERT OR IGNORE INTO hole_ranges(partition, lo, hi, first_seen) VALUES(?,?,?,?)",
                         (partition, lo, hi, now))
        return float(self._db.execute("SELECT first_seen FROM hole_ranges WHERE partition=? AND lo=?",
                                      (partition, lo)).fetchone()[0])

    def open_backfills(self, partition: str | None = None) -> list[tuple[int, int]]:
        q, args = "SELECT lo, hi FROM hole_ranges WHERE skipped=1", ()
        if partition:
            q, args = q + " AND partition=?", (partition,)
        return [(r[0], r[1]) for r in self._db.execute(q + " ORDER BY lo", args)]

    # --- partition status ---
    def stop(self, partition: str, reason: str) -> None:
        self._db.execute("INSERT INTO partition_status VALUES(?, 'stopped', ?) ON CONFLICT(partition) DO UPDATE "
                         "SET status='stopped', reason=excluded.reason", (partition, reason))

    def stopped(self) -> dict[str, str]:
        return {r[0]: r[1] for r in self._db.execute("SELECT partition, reason FROM partition_status WHERE "
                                                      "status='stopped'")}

    def is_stopped(self, partition: str) -> bool:
        return partition in self.stopped()

    def quarantine_batch(self, key: str, partition: str, reason: str, body: bytes) -> None:
        self._db.execute("INSERT OR REPLACE INTO batch_quarantine VALUES(?,?,?,?)", (key, partition, reason, body))
        self._db.execute("DELETE FROM pending_batch")

    # --- quarantined events / counters / meta ---
    def quarantined(self) -> list[tuple[str, int, str, str]]:
        return [(r[0], r[1], r[2], r[3]) for r in self._db.execute(
            "SELECT event_id, sequence, event_type, reason FROM quarantine ORDER BY sequence")]

    def counter(self, k: str) -> int:
        r = self._db.execute("SELECT n FROM counters WHERE k=?", (k,)).fetchone()
        return int(r[0]) if r else 0

    def meta(self, k: str, default: str | None = None) -> str | None:
        r = self._db.execute("SELECT v FROM meta WHERE k=?", (k,)).fetchone()
        return r[0] if r else default

    def set_meta(self, k: str, v: str) -> None:
        self._db.execute("INSERT OR REPLACE INTO meta VALUES(?,?)", (k, v))

    # --- pending batch / ack ---
    def pending(self) -> Pending | None:
        r = self._db.execute("SELECT partition, scan_mode, idem_key, body, delta FROM pending_batch").fetchone()
        return Pending(r[0], r[1], r[2], bytes(r[3]), json.loads(r[4])) if r else None

    def save_pending(self, p: Pending) -> None:
        self._db.execute("INSERT OR REPLACE INTO pending_batch VALUES(1,?,?,?,?,?)",
                         (p.partition, p.scan_mode, p.idem_key, p.body, json.dumps(p.delta)))

    def drop_pending(self) -> None:
        self._db.execute("DELETE FROM pending_batch")

    def commit_ack(self, p: Pending, ack: dict[str, Any]) -> None:
        d = p.delta
        self._db.execute("BEGIN IMMEDIATE")
        try:
            for event_id, seq, etype, reason in d.get("quarantined", []):
                self._db.execute("INSERT OR IGNORE INTO quarantine VALUES(?,?,?,?)", (event_id, seq, etype, reason))
            for k, n in d.get("counters", {}).items():
                self._db.execute("INSERT INTO counters VALUES(?,?) ON CONFLICT(k) DO UPDATE SET n=n+excluded.n", (k, n))
            for lo, hi in d.get("holes_skipped", []):
                self._db.execute("INSERT INTO hole_ranges VALUES(?,?,?,0.0,1) ON CONFLICT(partition, lo) DO UPDATE "
                                 "SET skipped=1, hi=excluded.hi", (p.partition, lo, hi))
            if "holes_unskipped_below" in d:  # grace-period notes below an advanced position are obsolete
                self._db.execute("DELETE FROM hole_ranges WHERE partition=? AND skipped=0 AND lo<=?",
                                 (p.partition, d["holes_unskipped_below"]))
            for seq in d.get("holes_cleared", []):  # a skipped hole finally exported: split its range
                for lo, hi in self._db.execute("SELECT lo, hi FROM hole_ranges WHERE partition=? AND skipped=1 "
                                               "AND lo<=? AND hi>=?", (p.partition, seq, seq)).fetchall():
                    self._db.execute("DELETE FROM hole_ranges WHERE partition=? AND lo=?", (p.partition, lo))
                    if lo < seq:
                        self._db.execute("INSERT INTO hole_ranges VALUES(?,?,?,0.0,1)", (p.partition, lo, seq - 1))
                    if seq < hi:
                        self._db.execute("INSERT INTO hole_ranges VALUES(?,?,?,0.0,1)", (p.partition, seq + 1, hi))
            for k, v in d.get("meta", {}).items():
                self._db.execute("INSERT OR REPLACE INTO meta VALUES(?,?)", (k, v))
            if p.scan_mode == "fast_poll":
                self._db.execute("INSERT INTO partition_cursor VALUES(?,?,?) ON CONFLICT(partition) DO UPDATE SET "
                                 "cursor=excluded.cursor, revision=excluded.revision",
                                 (p.partition, ack.get("current_cursor"), int(ack["cursor_revision"])))
            self._db.execute("DELETE FROM pending_batch")
            self._db.execute("COMMIT")
        except BaseException:
            self._db.execute("ROLLBACK")
            raise
