"""Bridge-side persistence for evaluation (plan 17.3.5): full `EvalReport`s, admissions, arm executions.

Stock `get_write` returns only the verdict and, after a `fail`, `get_proposal.last_eval` is null (the proposal
goes back to draft with `candidate_hash=None`). The bridge therefore stores the full report (or the
`gate_failed` payload) keyed `(proposal_id, eval_run_id | synthetic)`. Tables live in `pulso_bridge` of the
runtime database; the schema is created idempotently by `ensure_eval_schema` (L2 should call it next to
`ensure_schema`; the stores also call it lazily)."""

from __future__ import annotations

import hashlib
import json
import threading
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any, Protocol

import psycopg
from agent_core.domain import canonical_bytes

EVAL_DDL = (
    "CREATE SCHEMA IF NOT EXISTS pulso_bridge",
    "CREATE TABLE IF NOT EXISTS pulso_bridge.eval_reports (proposal_id text NOT NULL, report_key text NOT NULL, "
    "eval_run_id text, verdict text NOT NULL, gate_failed boolean NOT NULL, report jsonb NOT NULL, "
    "report_digest text NOT NULL, created_at timestamptz NOT NULL DEFAULT now(), "
    "PRIMARY KEY (proposal_id, report_key))",
    "CREATE TABLE IF NOT EXISTS pulso_bridge.eval_admissions (evaluation_context_ref text PRIMARY KEY, "
    "tenant_id text NOT NULL, proposal_id text NOT NULL, candidate_hash text NOT NULL, "
    "evaluation_attempt integer NOT NULL, state text NOT NULL, request_digest text NOT NULL, "
    "record jsonb NOT NULL, created_at timestamptz NOT NULL DEFAULT now(), "
    "UNIQUE (tenant_id, proposal_id, candidate_hash, evaluation_attempt))",
    "CREATE TABLE IF NOT EXISTS pulso_bridge.arm_executions (execution_id text PRIMARY KEY, "
    "idempotency_key text NOT NULL UNIQUE, request_digest text NOT NULL, status text NOT NULL, "
    "report jsonb, created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz NOT NULL DEFAULT now())",
)


def ensure_eval_schema(dsn: str) -> None:
    with psycopg.connect(dsn, autocommit=True) as conn:
        for stmt in EVAL_DDL:
            conn.execute(stmt)  # type: ignore[arg-type]


def digest_of(value: Any) -> str:
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


@dataclass(frozen=True)
class StoredReport:
    proposal_id: str
    report_key: str
    eval_run_id: str | None
    verdict: str
    gate_failed: bool
    report: dict[str, Any]
    report_digest: str


class ReportStore(Protocol):
    def put(self, proposal_id: str, report_key: str, eval_run_id: str | None, verdict: str, gate_failed: bool,
            report: dict[str, Any]) -> StoredReport: ...

    def get(self, proposal_id: str, report_key: str) -> StoredReport | None: ...

    def list_for(self, proposal_id: str) -> list[StoredReport]: ...


class InMemoryReportStore:
    def __init__(self) -> None:
        self._rows: dict[tuple[str, str], StoredReport] = {}
        self._lock = threading.Lock()

    def put(self, proposal_id: str, report_key: str, eval_run_id: str | None, verdict: str, gate_failed: bool,
            report: dict[str, Any]) -> StoredReport:
        row = StoredReport(proposal_id, report_key, eval_run_id, verdict, gate_failed, report, digest_of(report))
        with self._lock:
            return self._rows.setdefault((proposal_id, report_key), row)  # first write wins (idempotent)

    def get(self, proposal_id: str, report_key: str) -> StoredReport | None:
        with self._lock:
            return self._rows.get((proposal_id, report_key))

    def list_for(self, proposal_id: str) -> list[StoredReport]:
        with self._lock:
            return [r for (p, _), r in self._rows.items() if p == proposal_id]


class PgReportStore:
    def __init__(self, dsn: str) -> None:
        self._dsn = dsn
        ensure_eval_schema(dsn)

    def put(self, proposal_id: str, report_key: str, eval_run_id: str | None, verdict: str, gate_failed: bool,
            report: dict[str, Any]) -> StoredReport:
        digest = digest_of(report)
        with psycopg.connect(self._dsn, autocommit=True) as conn:
            conn.execute(
                "INSERT INTO pulso_bridge.eval_reports (proposal_id, report_key, eval_run_id, verdict, "
                "gate_failed, report, report_digest) VALUES (%s,%s,%s,%s,%s,%s::jsonb,%s) "
                "ON CONFLICT DO NOTHING",
                (proposal_id, report_key, eval_run_id, verdict, gate_failed, json.dumps(report), digest))
        stored = self.get(proposal_id, report_key)
        assert stored is not None
        return stored

    def get(self, proposal_id: str, report_key: str) -> StoredReport | None:
        with psycopg.connect(self._dsn, autocommit=True) as conn:
            row = conn.execute(
                "SELECT eval_run_id, verdict, gate_failed, report, report_digest FROM pulso_bridge.eval_reports "
                "WHERE proposal_id=%s AND report_key=%s", (proposal_id, report_key)).fetchone()
        return None if row is None else StoredReport(proposal_id, report_key, row[0], row[1], row[2], row[3], row[4])

    def list_for(self, proposal_id: str) -> list[StoredReport]:
        with psycopg.connect(self._dsn, autocommit=True) as conn:
            rows = conn.execute(
                "SELECT report_key, eval_run_id, verdict, gate_failed, report, report_digest FROM "
                "pulso_bridge.eval_reports WHERE proposal_id=%s ORDER BY created_at", (proposal_id,)).fetchall()
        return [StoredReport(proposal_id, r[0], r[1], r[2], r[3], r[4], r[5]) for r in rows]


# --- arm executions (single-flight, plan 17.3.5 step 1 / 8) -------------------------------------------------

@dataclass(frozen=True)
class ArmRow:
    execution_id: str
    idempotency_key: str
    request_digest: str
    status: str  # running | completed | candidate_failed | failed_infra | unknown
    report: dict[str, Any] | None


class ArmStore(Protocol):
    def begin(self, execution_id: str, key: str, request_digest: str) -> tuple[ArmRow, bool]:
        """Insert the single-flight row. Returns `(row, created)`; an existing row is returned untouched."""
        ...

    def finish(self, execution_id: str, status: str, report: dict[str, Any]) -> None: ...

    def get(self, execution_id: str) -> ArmRow | None: ...

    def get_by_key(self, key: str) -> ArmRow | None: ...


class InMemoryArmStore:
    def __init__(self) -> None:
        self._rows: dict[str, ArmRow] = {}
        self._lock = threading.Lock()

    def begin(self, execution_id: str, key: str, request_digest: str) -> tuple[ArmRow, bool]:
        with self._lock:
            for row in self._rows.values():
                if row.idempotency_key == key:
                    return row, False
            row = ArmRow(execution_id, key, request_digest, "running", None)
            self._rows[execution_id] = row
            return row, True

    def finish(self, execution_id: str, status: str, report: dict[str, Any]) -> None:
        with self._lock:
            old = self._rows[execution_id]
            self._rows[execution_id] = ArmRow(old.execution_id, old.idempotency_key, old.request_digest,
                                              status, report)

    def get(self, execution_id: str) -> ArmRow | None:
        with self._lock:
            return self._rows.get(execution_id)

    def get_by_key(self, key: str) -> ArmRow | None:
        with self._lock:
            return next((r for r in self._rows.values() if r.idempotency_key == key), None)


class PgArmStore:
    def __init__(self, dsn: str) -> None:
        self._dsn = dsn
        ensure_eval_schema(dsn)

    @staticmethod
    def _row(r: Any) -> ArmRow:
        return ArmRow(r[0], r[1], r[2], r[3], r[4])

    def begin(self, execution_id: str, key: str, request_digest: str) -> tuple[ArmRow, bool]:
        with psycopg.connect(self._dsn, autocommit=True) as conn:
            cur = conn.execute(
                "INSERT INTO pulso_bridge.arm_executions (execution_id, idempotency_key, request_digest, status) "
                "VALUES (%s,%s,%s,'running') ON CONFLICT DO NOTHING", (execution_id, key, request_digest))
            created = cur.rowcount == 1
        row = self.get_by_key(key)
        assert row is not None
        return row, created

    def finish(self, execution_id: str, status: str, report: dict[str, Any]) -> None:
        with psycopg.connect(self._dsn, autocommit=True) as conn:
            conn.execute("UPDATE pulso_bridge.arm_executions SET status=%s, report=%s::jsonb, updated_at=now() "
                         "WHERE execution_id=%s", (status, json.dumps(report), execution_id))

    def get(self, execution_id: str) -> ArmRow | None:
        with psycopg.connect(self._dsn, autocommit=True) as conn:
            r = conn.execute("SELECT execution_id, idempotency_key, request_digest, status, report FROM "
                             "pulso_bridge.arm_executions WHERE execution_id=%s", (execution_id,)).fetchone()
        return None if r is None else self._row(r)

    def get_by_key(self, key: str) -> ArmRow | None:
        with psycopg.connect(self._dsn, autocommit=True) as conn:
            r = conn.execute("SELECT execution_id, idempotency_key, request_digest, status, report FROM "
                             "pulso_bridge.arm_executions WHERE idempotency_key=%s", (key,)).fetchone()
        return None if r is None else self._row(r)


def utcnow() -> datetime:
    return datetime.now(UTC)
