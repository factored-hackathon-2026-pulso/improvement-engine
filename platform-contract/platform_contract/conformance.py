"""Conformance checks: row-level schema validation plus stream/invariant findings."""

from __future__ import annotations

import json
import sqlite3
from datetime import datetime

from jsonschema import Draft202012Validator, FormatChecker

from . import TABLES, assert_table_readable, classify_event_type, load_schema

_validators: dict[str, Draft202012Validator] = {}

# jsonschema only enforces "date-time" when an optional rfc3339 package is installed; register
# our own so the check can never silently turn into a no-op.
_formats = FormatChecker()


@_formats.checks("date-time", raises=ValueError)
def _is_datetime(value) -> bool:
    if not isinstance(value, str):
        return True
    datetime.fromisoformat(value.replace("Z", "+00:00"))
    return "T" in value


def _validator(table: str) -> Draft202012Validator:
    if table not in _validators:
        _validators[table] = Draft202012Validator(load_schema(table), format_checker=_formats)
    return _validators[table]


def validate_rows(table: str, rows) -> list[str]:
    """Return human-readable violations (empty list means conformant)."""
    v = _validator(table)
    errs = []
    for i, row in enumerate(rows):
        for e in v.iter_errors(row):
            path = ".".join(str(p) for p in e.absolute_path) or "<row>"
            # Never echo e.message: it embeds the offending value (free text, ids, notes).
            detail = e.message if e.validator in ("required", "additionalProperties") else (
                f"violates {e.validator}")
            errs.append(f"{table}[{i}].{path}: {detail}")
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


# --------------------------------------------------------------------------------------------------------------
# Revision 1.1.0: source_event discriminator (exporter_finding vs domain_event) and observation-level checks.

import hashlib  # noqa: E402

from . import LEGACY_EXPORTER_PREFIX, MAX_FINDING_DETAILS_BYTES, load_source_schema  # noqa: E402

_obs_validator: Draft202012Validator | None = None


def classify_source_event(se) -> str:
    """exporter_finding | domain_event (1.1.0 kinds), legacy_exporter_prefix | legacy_domain_event (1.0.0 shape, no
    `kind`), or unsupported. Anything unsupported must be quarantined by the consumer with an explicit reason."""
    if not isinstance(se, dict):
        return "unsupported"
    kind = se.get("kind")
    if kind in ("exporter_finding", "domain_event"):
        return kind
    if kind is not None:
        return "unsupported"
    et = se.get("event_type")
    if isinstance(et, str) and et:
        return "legacy_exporter_prefix" if et.startswith(LEGACY_EXPORTER_PREFIX) else "legacy_domain_event"
    return "unsupported"


def _identity(r) -> tuple:
    return (r.get("tenant_id"), r.get("source_id"), r.get("native_event_id"))


def validate_source_observations(records) -> list[str]:
    """Schema plus cross-field rules of the 1.1.0 observation view. Messages never echo row values."""
    global _obs_validator
    if _obs_validator is None:
        _obs_validator = Draft202012Validator(load_source_schema("source_observation"), format_checker=_formats)
    errs: list[str] = []
    for i, r in enumerate(records):
        n0 = len(errs)
        for e in _obs_validator.iter_errors(r):
            path = ".".join(str(p) for p in e.absolute_path) or "<row>"
            detail = e.message if e.validator in ("required", "additionalProperties") else f"violates {e.validator}"
            errs.append(f"source_observation[{i}].{path}: {detail}")
        if len(errs) > n0 or not isinstance(r, dict):
            continue
        se = r["source_event"]
        if se["tenant_id"] != r["tenant_id"]:
            errs.append(f"source_observation[{i}].source_event.tenant_id: differs from envelope")
        if se["kind"] == "exporter_finding":
            size = len(json.dumps(se["details"], separators=(",", ":"), ensure_ascii=False).encode("utf-8"))
            if size > MAX_FINDING_DETAILS_BYTES:
                errs.append(f"source_observation[{i}].source_event.details: exceeds {MAX_FINDING_DETAILS_BYTES} bytes")
        else:
            if se["event_id"] != r["native_event_id"]:
                errs.append(f"source_observation[{i}].native_event_id: differs from source_event.event_id")
            if r["source_sequence"] is None:
                errs.append(f"source_observation[{i}].source_sequence: required for a domain_event")
    return errs


def _digest(se) -> str:
    return hashlib.sha256(json.dumps(se, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()


def dedup_observations(records):
    """Dedup on (tenant_id, source_id, native_event_id): first wins. Returns (unique, duplicates); a duplicate is
    `duplicate_identical` (same source_event digest, safe to drop) or `identity_conflict` (same identity, different
    content: must be rejected by the receiver, never silently merged)."""
    seen: dict[tuple, str] = {}
    unique, dups = [], []
    for r in records:
        k, d = _identity(r), _digest(r["source_event"])
        if k not in seen:
            seen[k] = d
            unique.append(r)
        else:
            dups.append({"code": "duplicate_identical" if seen[k] == d else "identity_conflict", "identity": list(k)})
    return unique, dups


def check_observation_sequences(records) -> list[dict]:
    """Continuity is evaluated over domain rows only. exporter_finding rows are excluded: they may carry a null
    sequence or reuse the sequence of the row they describe, and may never fill a hole."""
    findings: list[dict] = []
    domain = sorted(
        (r for r in records if classify_source_event(r["source_event"]) in ("domain_event", "legacy_domain_event")),
        key=lambda r: r["source_sequence"])
    for prev, cur in zip(domain, domain[1:]):
        a, b = prev["source_sequence"], cur["source_sequence"]
        if b == a:
            findings.append({"code": "duplicate_sequence", "sequence": b})
        elif b != a + 1:
            findings.append({"code": "gap_suspected", "after": a, "next": b})
    for r in records:
        se = r["source_event"]
        if classify_source_event(se) == "exporter_finding" and r["source_sequence"] is not None \
                and r["source_sequence"] != se["described_source_sequence"]:
            findings.append({"code": "finding_sequence_mismatch", "native_event_id": r["native_event_id"]})
    return findings
