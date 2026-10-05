"""Portable DDL for the platform's 11-table model (SQLite today, Postgres-ready).

Types are mapped per dialect: timestamps are ISO-8601 UTC text in SQLite and TIMESTAMPTZ in
Postgres; JSON columns are TEXT / JSONB; booleans are INTEGER 0/1 / BOOLEAN. The simulator
assigns event_log.sequence explicitly (no AUTOINCREMENT), which keeps the DDL identical in shape.
The real platform creates its schema at startup without migrations; this mirrors the columns
listed in the Product artifact (commit a492bfa), nothing more.
"""

from __future__ import annotations

_TYPES = {
    "sqlite": {"TS": "TEXT", "JSON": "TEXT", "BOOL": "INTEGER", "BIG": "INTEGER"},
    "postgres": {"TS": "TIMESTAMPTZ", "JSON": "JSONB", "BOOL": "BOOLEAN", "BIG": "BIGINT"},
}

_TEMPLATE = [
    """CREATE TABLE customers (
  id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  country TEXT NOT NULL CHECK (country IN ('CO','MX','AR','BR')),
  city TEXT,
  locale TEXT NOT NULL CHECK (locale IN ('es-CO','es-MX','es-AR','pt-BR')),
  simulator {BOOL} NOT NULL DEFAULT {FALSE},
  suggestions {JSON}
)""",
    """CREATE TABLE staff (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  email TEXT NOT NULL UNIQUE,
  roles {JSON} NOT NULL,
  languages {JSON} NOT NULL,
  team TEXT,
  active {BOOL} NOT NULL DEFAULT {TRUE},
  version INTEGER NOT NULL DEFAULT 1
)""",
    """CREATE TABLE cases (
  id TEXT PRIMARY KEY,
  customer_id TEXT NOT NULL REFERENCES customers(id),
  channel TEXT NOT NULL CHECK (channel IN ('app_chat','web_chat','chat_app','chat_web','phone_inbound','phone_outbound','email')),
  language TEXT NOT NULL CHECK (language IN ('es','pt')),
  priority TEXT NOT NULL CHECK (priority IN ('none','low','medium','high','critical')),
  status TEXT NOT NULL CHECK (status IN ('queued','assigned','in_progress','closed','with_assistant')),
  opened_at {TS} NOT NULL,
  sla_due_at {TS} NOT NULL,
  first_response_at {TS},
  previous_case_id TEXT REFERENCES cases(id),
  assigned_analyst_id TEXT REFERENCES staff(id),
  assigned_at {TS},
  queued_at {TS},
  queue_label TEXT,
  last_sequence INTEGER NOT NULL DEFAULT 0,
  last_public_sequence INTEGER NOT NULL DEFAULT 0,
  last_message_at {TS},
  last_message_author_role TEXT,
  last_message_preview TEXT,
  last_turn_author_role TEXT,
  last_turn_preview TEXT,
  assignee_read_sequence INTEGER NOT NULL DEFAULT 0,
  unread_sequences {JSON} NOT NULL,
  search_text TEXT,
  closed_at {TS},
  closed_by_id TEXT,
  closed_by_role TEXT,
  close_reason TEXT CHECK (close_reason IS NULL OR close_reason IN
    ('resolved','customer_unresponsive','duplicate','out_of_scope','other')),
  close_note TEXT,
  rating_score INTEGER CHECK (rating_score IS NULL OR rating_score BETWEEN 1 AND 4),
  rated_at {TS},
  version INTEGER NOT NULL DEFAULT 1
)""",
    """CREATE TABLE turns (
  id TEXT PRIMARY KEY,
  case_id TEXT NOT NULL REFERENCES cases(id),
  sequence INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('message','routing','notice','transcript','note','email')),
  audience TEXT NOT NULL CHECK (audience IN ('everyone','staff')),
  author_role TEXT NOT NULL CHECK (author_role IN ('customer','analyst','system','assistant')),
  author_id TEXT,
  text TEXT NOT NULL,
  language TEXT NOT NULL,
  created_at {TS} NOT NULL,
  client_message_id TEXT,
  UNIQUE (case_id, sequence)
)""",
    """CREATE TABLE assignments (
  id TEXT PRIMARY KEY,
  case_id TEXT NOT NULL REFERENCES cases(id),
  staff_id TEXT NOT NULL REFERENCES staff(id),
  reason TEXT NOT NULL CHECK (reason IN ('language_least_loaded','queue_drained','manual','outbound_call','assistant_handoff')),
  policy_rule_id TEXT,
  open_cases_at_assignment INTEGER NOT NULL,
  strategy TEXT NOT NULL,
  assigned_by_role TEXT NOT NULL,
  assigned_by_id TEXT,
  waited_seconds INTEGER,
  previous_staff_id TEXT,
  paused_override {BOOL} NOT NULL DEFAULT {FALSE},
  assigned_at {TS} NOT NULL
)""",
    """CREATE TABLE customer_case_slots (
  customer_id TEXT PRIMARY KEY REFERENCES customers(id),
  open_case_id TEXT REFERENCES cases(id),
  version INTEGER NOT NULL DEFAULT 1
)""",
    # Credential / access tables: exist in the real platform and must never be read by Pulso.
    # The simulator fills them with obvious placeholders (FAKE-*) so exporters can prove refusal.
    """CREATE TABLE login_accounts (
  staff_id TEXT PRIMARY KEY REFERENCES staff(id),
  password_hash TEXT NOT NULL,
  failed_attempts INTEGER NOT NULL DEFAULT 0,
  locked_until {TS},
  last_login_at {TS}
)""",
    """CREATE TABLE mfa_challenges (
  id TEXT PRIMARY KEY,
  staff_id TEXT NOT NULL REFERENCES staff(id),
  issued_at {TS} NOT NULL,
  expires_at {TS} NOT NULL,
  max_attempts INTEGER NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL,
  verified_at {TS},
  method TEXT NOT NULL
)""",
    """CREATE TABLE staff_sessions (
  id TEXT PRIMARY KEY,
  staff_id TEXT NOT NULL REFERENCES staff(id),
  issued_at {TS} NOT NULL,
  expires_at {TS} NOT NULL,
  mfa_method TEXT,
  ended_at {TS},
  end_reason TEXT
)""",
    """CREATE TABLE analyst_availability (
  staff_id TEXT PRIMARY KEY REFERENCES staff(id),
  status TEXT NOT NULL CHECK (status IN ('available','paused')),
  since {TS} NOT NULL
)""",
    """CREATE TABLE event_log (
  sequence {BIG} PRIMARY KEY,
  event_id TEXT NOT NULL UNIQUE,
  event_type TEXT NOT NULL,
  entity TEXT,
  entity_id TEXT,
  case_id TEXT,
  actor_role TEXT,
  actor_id TEXT,
  event_time {TS} NOT NULL,
  ingested_at {TS} NOT NULL,
  payload {JSON} NOT NULL
)""",
    # Artifact: client_message_id "evita duplicados si se reenvia (unico por autor)".
    "CREATE UNIQUE INDEX ux_turns_client_msg ON turns (author_id, client_message_id) "
    "WHERE client_message_id IS NOT NULL",
    "CREATE INDEX ix_turns_case ON turns (case_id, sequence)",
    "CREATE INDEX ix_assignments_case ON assignments (case_id)",
    "CREATE INDEX ix_cases_customer ON cases (customer_id)",
    "CREATE INDEX ix_event_log_case ON event_log (case_id)",
]

# SQLite-only guards that make the append-only tables observable in tests; the real platform
# enforces append-only in application code, Postgres deployments would use triggers or grants.
APPEND_ONLY_TABLES = ("turns", "assignments", "event_log")


def render_ddl(dialect: str = "sqlite") -> list[str]:
    t = dict(_TYPES[dialect])
    t["TRUE"] = "1" if dialect == "sqlite" else "TRUE"
    t["FALSE"] = "0" if dialect == "sqlite" else "FALSE"
    return [s.format(**t) for s in _TEMPLATE]


def append_only_guards_sqlite() -> list[str]:
    out = []
    for tb in APPEND_ONLY_TABLES:
        for op in ("UPDATE", "DELETE"):
            out.append(
                f"CREATE TRIGGER {tb}_no_{op.lower()} BEFORE {op} ON {tb} "
                f"BEGIN SELECT RAISE(ABORT, '{tb} is append-only'); END"
            )
    return out


def append_only_guards_postgres() -> list[str]:
    """Postgres equivalent of the SQLite guards (the real platform enforces this in code)."""
    out = [
        "CREATE FUNCTION forbid_mutation() RETURNS trigger LANGUAGE plpgsql AS "
        "$$ BEGIN RAISE EXCEPTION '% is append-only', TG_TABLE_NAME; END $$"
    ]
    for tb in APPEND_ONLY_TABLES:
        out.append(f"CREATE TRIGGER {tb}_append_only BEFORE UPDATE OR DELETE ON {tb} "
                   "FOR EACH ROW EXECUTE FUNCTION forbid_mutation()")
    return out
