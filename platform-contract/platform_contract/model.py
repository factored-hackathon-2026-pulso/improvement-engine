"""Single source of truth for the platform_live contract (Phase 1 profile).

Derived from the Product team's data-model artifact (see ARTIFACT_STAMP). The JSON Schemas
and the event catalog under platform-contract/ are generated from this module; a drift test
fails when the committed files differ from what this module generates.
"""

from __future__ import annotations

CONTRACT_VERSION = "1.0.0"
PROFILE = "platform_live.phase1"

ARTIFACT_STAMP = {
    "artifact_id": "BWx4saeWfYsLbQEbkNKMPg",
    "title": "Modelo de datos · Plataforma CC",
    "commit": "a492bfa",
    "slices": "0-3",
    "captured": "2026-10-03",
}

DENIED_TABLES = ("login_accounts", "mfa_challenges", "staff_sessions")

# Columns that exist in the real platform but are never read (credentials, names, free-text
# derivatives, demo text). Kept explicit so the denial is testable and visible.
DENIED_COLUMNS = {
    "cases": ("search_text", "last_message_preview", "last_turn_preview"),
    "customers": ("display_name", "suggestions"),
    "staff_dimensions": ("name", "email"),
}

ID_PATTERNS = {
    "case": r"^CASE-.+$",
    "turn": r"^TRN-.+$",
    "assignment": r"^ASG-.+$",
    "customer": r"^CUS-.+$",
    "staff": r"^STF-.+$",
    "actor": r"^(CUS|STF)-.+$",
    "event": r"^EVT-.+$",
}

ROLES = ["customer", "analyst", "supervisor", "admin", "system"]


def _c(name, kind, nullable=False, **extra):
    """Column spec: (name, kind, nullable, extras). kind: string|integer|boolean|datetime|array|object."""
    return (name, kind, nullable, extra)


TABLES: dict[str, dict] = {
    "cases": {
        "source_table": "cases",
        "mutable": True,
        "primary_key": ["id"],
        "columns": [
            _c("id", "string", pattern=ID_PATTERNS["case"]),
            _c("customer_id", "string", pattern=ID_PATTERNS["customer"]),
            _c("channel", "string", enum=["app_chat", "web_chat"]),
            _c("language", "string", enum=["es", "pt"]),
            _c("priority", "string", enum=["low", "medium", "high"]),
            _c("status", "string", enum=["queued", "assigned", "in_progress", "closed"]),
            _c("opened_at", "datetime"),
            _c("sla_due_at", "datetime"),
            _c("first_response_at", "datetime", True),
            _c("previous_case_id", "string", True, pattern=ID_PATTERNS["case"]),
            _c("assigned_analyst_id", "string", True, pattern=ID_PATTERNS["staff"]),
            _c("assigned_at", "datetime", True),
            _c("queued_at", "datetime", True),
            _c("queue_label", "string", True),
            _c("last_sequence", "integer", minimum=0),
            _c("last_public_sequence", "integer", minimum=0),
            _c("last_message_at", "datetime", True),
            _c("last_message_author_role", "string", True, enum=ROLES),
            _c("last_turn_author_role", "string", True, enum=ROLES),
            _c("assignee_read_sequence", "integer", minimum=0),
            _c("unread_sequences", "array", items={"type": "integer"}),
            _c("closed_at", "datetime", True),
            _c("closed_by_id", "string", True, pattern=ID_PATTERNS["staff"]),
            _c("closed_by_role", "string", True, enum=ROLES),
            _c(
                "close_reason", "string", True,
                enum=["resolved", "customer_unresponsive", "duplicate", "out_of_scope", "other"],
            ),
            _c("close_note", "string", True, maxLength=500, sensitive_text=True),
            _c("version", "integer", minimum=1),
        ],
    },
    "turns": {
        "source_table": "turns",
        "mutable": False,
        "primary_key": ["id"],
        "columns": [
            _c("id", "string", pattern=ID_PATTERNS["turn"]),
            _c("case_id", "string", pattern=ID_PATTERNS["case"]),
            _c("sequence", "integer", minimum=1),
            _c("kind", "string", enum=["message", "routing", "notice"]),
            _c("audience", "string", enum=["everyone", "staff"]),
            _c("author_role", "string", enum=["customer", "analyst", "system"]),
            _c("author_id", "string", True, pattern=ID_PATTERNS["actor"]),
            _c("text", "string", sensitive_text=True),
            _c("language", "string", enum=["es", "pt"]),
            _c("created_at", "datetime"),
            _c("client_message_id", "string", True),
        ],
    },
    "assignments": {
        "source_table": "assignments",
        "mutable": False,
        "primary_key": ["id"],
        "columns": [
            _c("id", "string", pattern=ID_PATTERNS["assignment"]),
            _c("case_id", "string", pattern=ID_PATTERNS["case"]),
            _c("staff_id", "string", pattern=ID_PATTERNS["staff"]),
            _c("reason", "string", enum=["language_least_loaded", "queue_drained", "manual"]),
            _c("policy_rule_id", "string", True),
            _c("open_cases_at_assignment", "integer", minimum=0),
            _c("strategy", "string"),
            _c("assigned_by_role", "string", enum=["system", "supervisor", "admin"]),
            _c("assigned_by_id", "string", True),
            _c("waited_seconds", "integer", True, minimum=0),
            _c("previous_staff_id", "string", True, pattern=ID_PATTERNS["staff"]),
            _c("paused_override", "boolean"),
            _c("assigned_at", "datetime"),
        ],
    },
    "customer_case_slots": {
        "source_table": "customer_case_slots",
        "mutable": True,
        "primary_key": ["customer_id"],
        "columns": [
            _c("customer_id", "string", pattern=ID_PATTERNS["customer"]),
            _c("open_case_id", "string", True, pattern=ID_PATTERNS["case"]),
            _c("version", "integer", minimum=1),
        ],
    },
    "customers": {
        "source_table": "customers",
        "mutable": False,
        "primary_key": ["id"],
        "columns": [
            _c("id", "string", pattern=ID_PATTERNS["customer"]),
            _c("country", "string", enum=["CO", "MX", "AR", "BR"]),
            _c("city", "string", True),
            _c("locale", "string", enum=["es-CO", "es-MX", "es-AR", "pt-BR"]),
            _c("simulator", "boolean"),
        ],
    },
    "event_log": {
        "source_table": "event_log",
        "mutable": False,
        "primary_key": ["sequence"],
        "columns": [
            _c("sequence", "integer", minimum=1),
            _c("event_id", "string", pattern=ID_PATTERNS["event"]),
            _c("event_type", "string", minLength=1),
            _c("entity", "string", True),
            _c("entity_id", "string", True),
            _c("case_id", "string", True, pattern=ID_PATTERNS["case"]),
            _c("actor_role", "string", True, enum=ROLES),
            _c("actor_id", "string", True),
            _c("event_time", "datetime"),
            _c("ingested_at", "datetime"),
            _c("payload", "object"),
        ],
    },
    # Minimal staff dimensions: roles, languages, team, active. Names/emails never read.
    # `team` is the Phase 1 column; `team_id` is the announced replacement (both optional so
    # the contract survives the teams evolution).
    "staff_dimensions": {
        "source_table": "staff",
        "mutable": True,
        "primary_key": ["id"],
        "optional": ["team", "team_id"],
        "columns": [
            _c("id", "string", pattern=ID_PATTERNS["staff"]),
            _c("roles", "array", items={"type": "string", "enum": ["analyst", "supervisor", "admin"]}),
            _c("languages", "array", items={"type": "string", "enum": ["es", "pt"]}),
            _c("team", "string", True),
            _c("team_id", "string", True),
            _c("active", "boolean"),
            _c("version", "integer", minimum=1),
        ],
    },
}

# Event catalog. status: admitted (ingest), denied (known, never ingest), planned (announced,
# not yet admitted: quarantined like unknown until the versioned allow-list admits it).
EVENT_CATALOG_VERSION = "1.0.0"
EVENT_TYPES = [
    ("case.opened", "cases", "case", "admitted"),
    ("case.queued", "cases", "case", "admitted"),
    ("case.assigned", "cases", "case", "admitted"),
    ("case.status_changed", "cases", "case", "admitted"),
    ("case.read", "cases", "case", "admitted"),
    ("case.first_responded", "cases", "case", "admitted"),
    ("case.closed", "cases", "case", "admitted"),
    ("case.viewed", "cases", "case", "admitted"),
    ("turn.created", "messages", "turn", "admitted"),
    ("staff.availability_changed", "team", "staff", "admitted"),
    ("auth.login_failed", "access", "staff", "admitted"),
    ("auth.account_locked", "access", "staff", "admitted"),
    ("auth.session_started", "access", "staff", "admitted"),
    ("auth.session_ended", "access", "staff", "admitted"),
    ("auth.password_accepted", "access", "staff", "denied"),
    ("auth.mfa_challenge_issued", "access", "staff", "denied"),
    ("auth.mfa_failed", "access", "staff", "denied"),
    ("customer.session_started", "access", "customer", "denied"),
]
# Announced by Product (administration slice, uncommitted at capture): staff.* and team.*.
PLANNED_EVENT_PREFIXES = ("staff.", "team.")
PLANNED_TABLES = ("teams", "admin_roster")

SQL_TYPES = {
    "string": "TEXT", "integer": "INTEGER", "boolean": "INTEGER",
    "datetime": "TEXT", "array": "TEXT", "object": "TEXT",
}
