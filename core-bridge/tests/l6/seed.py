"""Seeding helpers for the L6 exporter tests. They write through the pinned agent-core adapters (real chain
functions, real `audit_events`/`reg_events`/`outbox` tables on a real PG16); nothing here is mocked."""

from __future__ import annotations

from typing import Any

import psycopg
from agent_core.adapters.postgres_audit import PgAuditEvents
from agent_core.audit.chain import chain_events
from agent_core.domain import AnyEvent, dumps
from pydantic import TypeAdapter

_ANY: TypeAdapter[Any] = TypeAdapter(AnyEvent)


def make_event(run_id: str, i: int, kind: str, ts: str) -> Any:
    base: dict[str, Any] = {"run_id": run_id, "release": "rel1", "ts": ts, "event_id": f"{run_id}-e{i}", "type": kind}
    payloads = {
        "run_started": {"agent": {"id": "a", "version": "1.0.0"}, "mode": "task", "principal_type": "customer",
                       "locale": "es", "reportable_attrs": {"secret_canary": "CANARY-ATTR"}},
        "node_entered": {"flow": {"id": "f", "version": "1.0.0"}, "node_id": "n", "node_type": "x",
                         "resume_kind": "none"},
        "run_closed": {"outcome": "resolved", "closed_by": "flow"},
        "handoff_resolved": {"handoff_ref": "h1", "resolution_code": "done", "handoff_quality": "useful",
                             "reader_type": "customer"},
    }
    return _ANY.validate_python({**base, "payload": payloads[kind]})


def append_events(conn: psycopg.Connection[Any], run_id: str, kinds: list[str], ts: str = "2026-10-03T01:00:00Z",
                  start_index: int = 0) -> list[Any]:
    audit = PgAuditEvents(conn)
    last = audit.last_event(run_id)
    raw = [make_event(run_id, start_index + i, k, ts) for i, k in enumerate(kinds)]
    chained = chain_events(run_id, raw, last)
    audit.append_events(run_id, chained)
    conn.commit()
    return chained


def seed_run(dsn: str, run_id: str, middle: int = 1, close: bool = True, ts: str = "2026-10-03T01:00:00Z") -> list[Any]:
    kinds = ["run_started", *(["node_entered"] * middle), *(["run_closed"] if close else [])]
    with psycopg.connect(dsn) as conn:
        return append_events(conn, run_id, kinds, ts)


def insert_reg_event(conn: psycopg.Connection[Any], kind: str = "published", release_id: str | None = "rel-1") -> None:
    ev = {"type": kind, "actor": "a", "principal_type": "human", "origin": None, "proposal_id": "p1",
          "candidate_hash": None, "release_id": release_id, "at": "2026-10-03T01:00:00Z"}
    conn.execute("INSERT INTO reg_events (event_json) VALUES (%s)", (dumps(ev),))


def insert_outbox(conn: psycopg.Connection[Any], message_id: str, run_id: str = "r-ob") -> None:
    msg = {"message_id": message_id, "type": "handoff_created", "run_id": run_id, "created_at": "2026-10-03T01:00:00Z",
           "payload": {"handoff_ref": "h1", "run_id": run_id, "target_queue": "q", "priority": "p",
                       "reason_code": "rule:x", "language": "es"}}
    conn.execute("INSERT INTO outbox (message_id, message_json) VALUES (%s, %s)", (message_id, dumps(msg)))
