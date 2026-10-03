"""Minimal SQLite fixture of the Phase 1 platform model (11 tables). Own double: used until/besides platform-sim."""

from __future__ import annotations

import json
import sqlite3
from pathlib import Path

DDL = """
CREATE TABLE customers(id TEXT PRIMARY KEY, display_name TEXT, simulator INTEGER NOT NULL DEFAULT 0);
CREATE TABLE staff(id TEXT PRIMARY KEY, name TEXT, email TEXT, roles TEXT, languages TEXT, team TEXT, active INTEGER,
  version INTEGER NOT NULL DEFAULT 1);
CREATE TABLE login_accounts(staff_id TEXT PRIMARY KEY, password_hash TEXT, failed_attempts INTEGER, locked_until TEXT);
CREATE TABLE mfa_challenges(challenge_id TEXT PRIMARY KEY, staff_id TEXT, code_hash TEXT);
CREATE TABLE staff_sessions(session_id TEXT PRIMARY KEY, staff_id TEXT, token_hash TEXT);
CREATE TABLE cases(id TEXT PRIMARY KEY, customer_id TEXT, channel TEXT, language TEXT, priority TEXT,
  status TEXT, close_reason TEXT, close_note TEXT, assigned_analyst_id TEXT, previous_case_id TEXT,
  sla_due_at TEXT, opened_at TEXT, closed_at TEXT, version INTEGER NOT NULL DEFAULT 1);
CREATE TABLE turns(id TEXT PRIMARY KEY, case_id TEXT, sequence INTEGER, kind TEXT, author_role TEXT, text TEXT, created_at TEXT);
CREATE TABLE assignments(id TEXT PRIMARY KEY, case_id TEXT, staff_id TEXT, reason TEXT, assigned_at TEXT);
CREATE TABLE customer_case_slots(customer_id TEXT PRIMARY KEY, open_case_id TEXT, version INTEGER NOT NULL DEFAULT 1);
CREATE TABLE suggestions(suggestion_id TEXT PRIMARY KEY, case_id TEXT, text TEXT);
CREATE TABLE event_log(sequence INTEGER PRIMARY KEY, event_id TEXT NOT NULL UNIQUE, event_type TEXT NOT NULL,
  entity TEXT, entity_id TEXT, case_id TEXT, actor_role TEXT, actor_id TEXT, event_time TEXT NOT NULL, ingested_at TEXT NOT NULL, payload TEXT NOT NULL);
"""


def make_db(path: Path) -> sqlite3.Connection:
    db = sqlite3.connect(str(path))
    db.executescript(DDL)
    db.execute("INSERT INTO login_accounts VALUES('S1','$2b$secret-hash',0,NULL)")
    db.execute("INSERT INTO staff VALUES('S1','Ana Real Name','ana@example.invalid','[\"analyst\"]','[\"es\",\"pt\"]','T1',1,1)")
    db.execute("INSERT INTO customers VALUES('CUS-1','Real Customer',0)")
    db.execute("INSERT INTO customers VALUES('CUS-SIM','Demo Customer',1)")
    db.execute("INSERT INTO cases(id,customer_id,channel,language,priority,status,close_reason,opened_at,closed_at) "
               "VALUES('CASE-1','CUS-1','web_chat','es','high','closed','resolved','2026-03-01T10:00:00Z','2026-03-01T12:00:00Z')")
    db.commit()
    return db


def add_event(db: sqlite3.Connection, seq: int, event_type: str, *, event_time: str = "2026-03-01T10:00:00Z",
              ingested_at: str | None = None, case_id: str | None = "CASE-1", payload: dict | None = None,
              event_id: str | None = None) -> None:
    db.execute("INSERT INTO event_log(sequence,event_id,event_type,entity,entity_id,case_id,actor_role,actor_id,event_time,"
               "ingested_at,payload) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
               (seq, event_id or f"EVT-{seq}", event_type, "case", case_id, case_id, "analyst", "S1", event_time,
                ingested_at or event_time, json.dumps(payload or {})))
    db.commit()
