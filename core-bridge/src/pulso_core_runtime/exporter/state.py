"""Exporter local state: SQLite (WAL) on its own volume, never inside Core databases.

Tables: run_head, ledger (exported run/seq/hash), partition_cursor, hole_ranges (inclusive seq ranges), partition_status, pending_batch.
`commit_ack` applies a batch's delta in ONE transaction together with the pending-batch delete."""

from __future__ import annotations

import json
import sqlite3
from dataclasses import dataclass
from pathlib import Path
from typing import Any

SCHEMA = """
CREATE TABLE IF NOT EXISTS run_head(run_id TEXT PRIMARY KEY, last_seq INTEGER NOT NULL, last_hash TEXT NOT NULL,
  closed INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL DEFAULT 'ok', reason TEXT);
CREATE TABLE IF NOT EXISTS ledger(run_id TEXT NOT NULL, seq INTEGER NOT NULL, hash TEXT NOT NULL,
  PRIMARY KEY(run_id, seq));
CREATE TABLE IF NOT EXISTS partition_cursor(partition TEXT PRIMARY KEY, cursor TEXT, revision INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS hole_ranges(partition TEXT NOT NULL, lo INTEGER NOT NULL, hi INTEGER NOT NULL,
  first_seen REAL NOT NULL, skipped INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(partition, lo));
CREATE TABLE IF NOT EXISTS partition_status(partition TEXT PRIMARY KEY, status TEXT NOT NULL, reason TEXT);
CREATE TABLE IF NOT EXISTS pending_batch(id INTEGER PRIMARY KEY CHECK(id=1), partition TEXT NOT NULL,
  scan_mode TEXT NOT NULL, idem_key TEXT NOT NULL, body BLOB NOT NULL, delta TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS quarantine(idem_key TEXT PRIMARY KEY, partition TEXT, reason TEXT, body BLOB);
CREATE TABLE IF NOT EXISTS meta(k TEXT PRIMARY KEY, v TEXT NOT NULL);
"""


@dataclass(frozen=True)
class RunHead:
    run_id: str
    last_seq: int
    last_hash: str
    closed: bool
    status: str
    reason: str | None


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

    # --- runs / ledger ---
    def run(self, run_id: str) -> RunHead | None:
        r = self._db.execute("SELECT run_id,last_seq,last_hash,closed,status,reason FROM run_head WHERE run_id=?",
                             (run_id,)).fetchone()
        return RunHead(r[0], r[1], r[2], bool(r[3]), r[4], r[5]) if r else None

    def runs(self) -> list[RunHead]:
        return [RunHead(r[0], r[1], r[2], bool(r[3]), r[4], r[5]) for r in self._db.execute(
            "SELECT run_id,last_seq,last_hash,closed,status,reason FROM run_head ORDER BY run_id")]

    def set_run_status(self, run_id: str, last_seq: int, last_hash: str, closed: bool, status: str,
                       reason: str | None) -> None:
        self._db.execute(
            "INSERT INTO run_head VALUES(?,?,?,?,?,?) ON CONFLICT(run_id) DO UPDATE SET status=excluded.status, "
            "reason=excluded.reason", (run_id, last_seq, last_hash, int(closed), status, reason))

    def ledger_hash(self, run_id: str, seq: int) -> str | None:
        r = self._db.execute("SELECT hash FROM ledger WHERE run_id=? AND seq=?", (run_id, seq)).fetchone()
        return r[0] if r else None

    # --- cursors ---
    def cursor(self, partition: str) -> tuple[str | None, int] | None:
        r = self._db.execute("SELECT cursor, revision FROM partition_cursor WHERE partition=?",
                             (partition,)).fetchone()
        return (r[0], r[1]) if r else None

    def set_cursor(self, partition: str, cursor: str | None, revision: int) -> None:
        self._db.execute("INSERT INTO partition_cursor VALUES(?,?,?) ON CONFLICT(partition) DO UPDATE SET "
                         "cursor=excluded.cursor, revision=excluded.revision", (partition, cursor, revision))

    # --- holes (inclusive seq ranges: a bigserial jump of billions is one row, never enumerated) ---
    def note_hole_range(self, partition: str, lo: int, hi: int, now: float) -> float:
        self._db.execute("INSERT OR IGNORE INTO hole_ranges(partition, lo, hi, first_seen) VALUES(?,?,?,?)",
                         (partition, lo, hi, now))
        return float(self._db.execute("SELECT first_seen FROM hole_ranges WHERE partition=? AND lo=?",
                                      (partition, lo)).fetchone()[0])

    def skipped_ranges(self, partition: str) -> list[tuple[int, int]]:
        return [(r[0], r[1]) for r in self._db.execute(
            "SELECT lo, hi FROM hole_ranges WHERE partition=? AND skipped=1 ORDER BY lo", (partition,))]

    def any_skipped(self) -> bool:
        return self._db.execute("SELECT 1 FROM hole_ranges WHERE skipped=1 LIMIT 1").fetchone() is not None

    # --- partition status ---
    def stop(self, partition: str, reason: str) -> None:
        self._db.execute("INSERT INTO partition_status VALUES(?, 'stopped', ?) ON CONFLICT(partition) DO UPDATE "
                         "SET status='stopped', reason=excluded.reason", (partition, reason))

    def stopped(self) -> dict[str, str]:
        return {r[0]: r[1] for r in self._db.execute(
            "SELECT partition, reason FROM partition_status WHERE status='stopped'")}

    def is_stopped(self, partition: str) -> bool:
        return partition in self.stopped()

    def quarantine(self, key: str, partition: str, reason: str, body: bytes) -> None:
        self._db.execute("INSERT OR REPLACE INTO quarantine VALUES(?,?,?,?)", (key, partition, reason, body))
        self._db.execute("DELETE FROM pending_batch")

    # --- meta ---
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
            for run_id, h in d.get("runs", {}).items():
                self._db.execute(
                    "INSERT INTO run_head VALUES(?,?,?,?, 'ok', NULL) ON CONFLICT(run_id) DO UPDATE SET "
                    "last_seq=MAX(last_seq, excluded.last_seq), last_hash=CASE WHEN excluded.last_seq>=last_seq "
                    "THEN excluded.last_hash ELSE last_hash END, closed=MAX(closed, excluded.closed)",
                    (run_id, h["last_seq"], h["last_hash"], int(h["closed"])))
            for run_id, seq, hsh in d.get("ledger", []):
                self._db.execute("INSERT OR IGNORE INTO ledger VALUES(?,?,?)", (run_id, seq, hsh))
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
            if p.scan_mode == "fast_poll":
                self._db.execute("INSERT INTO partition_cursor VALUES(?,?,?) ON CONFLICT(partition) DO UPDATE SET "
                                 "cursor=excluded.cursor, revision=excluded.revision",
                                 (p.partition, ack.get("current_cursor"), int(ack["cursor_revision"])))
            self._db.execute("DELETE FROM pending_batch")
            self._db.execute("COMMIT")
        except BaseException:
            self._db.execute("ROLLBACK")
            raise
