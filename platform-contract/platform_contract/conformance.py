"""Conformance checks: row-level schema validation plus stream/invariant findings."""

from __future__ import annotations

import json
import sqlite3
from datetime import datetime

from jsonschema import Draft202012Validator, FormatChecker

from . import TABLES, assert_table_readable, classify_event_type, load_schema

_validators: dict[str, Draft202012Validator] = {}


def _validator(table: str) -> Draft202012Validator:
    if table not in _validators:
        _validators[table] = Draft202012Validator(load_schema(table), format_checker=FormatChecker())
    return _validators[table]


def validate_rows(table: str, rows) -> list[str]:
    """Return human-readable violations (empty list means conformant)."""
    v = _validator(table)
    errs = []
    for i, row in enumerate(rows):
        for e in v.iter_errors(row):
            path = ".".join(str(p) for p in e.absolute_path) or "<row>"
            errs.append(f"{table}[{i}].{path}: {e.message}")
    return errs


def _ts(s: str) -> datetime:
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


def check_event_stream(events, late_after_seconds: float | None = None) -> list[dict]:
    """Findings over an event_log extract: gaps, unknown/planned types, late events."""
    findings = []
    ordered = sorted(events, key=lambda e: e["sequence"])
    for prev, cur in zip(ordered, ordered[1:]):
        if cur["sequence"] != prev["sequence"] + 1:
            findings.append({"code": "gap_suspected", "after": prev["sequence"], "next": cur["sequence"]})
    for e in ordered:
        cls = classify_event_type(e["event_type"])
        if cls == "unknown":
            findings.append({"code": "unknown_event_type", "sequence": e["sequence"],
                             "event_type": e["event_type"]})
        elif cls == "planned":
            findings.append({"code": "planned_event_type", "sequence": e["sequence"],
                             "event_type": e["event_type"]})
        if late_after_seconds is not None:
            lag = (_ts(e["ingested_at"]) - _ts(e["event_time"])).total_seconds()
            if lag > late_after_seconds:
                findings.append({"code": "late_event", "sequence": e["sequence"], "lag_seconds": lag})
    return findings


def check_turn_sequences(turns) -> list[dict]:
    findings = []
    by_case: dict[str, list[int]] = {}
    for t in turns:
        by_case.setdefault(t["case_id"], []).append(t["sequence"])
    for case_id, seqs in by_case.items():
        seqs.sort()
        if seqs != list(range(1, len(seqs) + 1)):
            findings.append({"code": "turn_gap_suspected", "case_id": case_id})
    return findings


_JSON_COLS = {"unread_sequences", "roles", "languages", "payload"}
_BOOL_COLS = {"paused_override", "simulator", "active"}


def read_table(conn: sqlite3.Connection, table: str) -> list[dict]:
    """Project a SQLite table through the contract: allowed columns only, typed."""
    assert_table_readable(table)
    spec = TABLES[table]
    cols = [c[0] for c in spec["columns"]]
    present = {r[1] for r in conn.execute(f"PRAGMA table_info({spec['source_table']})")}
    use = [c for c in cols if c in present]
    cur = conn.execute(f"SELECT {', '.join(use)} FROM {spec['source_table']} ORDER BY 1")
    rows = []
    for values in cur:
        d = dict(zip(use, values))
        for c in list(d):
            if c in _JSON_COLS and isinstance(d[c], str):
                d[c] = json.loads(d[c])
            if c in _BOOL_COLS and d[c] is not None:
                d[c] = bool(d[c])
        rows.append(d)
    return rows


def check_database(conn: sqlite3.Connection) -> dict:
    """Full conformance of a platform database: schema violations plus invariant findings."""
    violations: list[str] = []
    data = {}
    for t in TABLES:
        data[t] = read_table(conn, t)
        violations += validate_rows(t, data[t])
    findings = check_event_stream(data["event_log"]) + check_turn_sequences(data["turns"])
    return {"violations": violations, "findings": findings, "rows": {t: len(v) for t, v in data.items()}}
