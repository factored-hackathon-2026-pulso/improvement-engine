"""Single source of truth for the platform_live contract (Phase 1 profile).

Derived from the Product team's data-model artifact (see ARTIFACT_STAMP). The JSON Schemas
and the event catalog under platform-contract/ are generated from this module; a drift test
fails when the committed files differ from what this module generates.
"""

from __future__ import annotations

CONTRACT_VERSION = "1.3.0"
PREVIOUS_CONTRACT_VERSION = "1.2.0"
PROFILE = "platform_live.phase1"

ARTIFACT_STAMP = {
    "artifact_id": "BWx4saeWfYsLbQEbkNKMPg",
    "title": "Modelo de datos · Plataforma CC",
    # 1.2.0 was derived from the platform CODE at eeb73a8; 1.3.0 (additive) from main 5261ecf (2026-10-05: the AI
    # maturity slices 18-23, the copilot suggestions of ADR 0005, `release`/`turn_id` on the AI events, PRs #27/#28).
    # Both read tables.py, domain/cases/values.py, application/audit/catalog.py, the domain events and backend/openapi.json.
    # The platform's DATA_MODEL.md is stale (says "slices 0 to 12, people only").
    "commit": "5261ecf",
    "previous_commit": "eeb73a8",
    "slices": "0-23 (assistant S13/S14, copilot S15, builder S16, hardening S17, maturity S18-S23, engine signals)",
    "captured": "2026-10-05",
    # 1.1.0 cited a492bfa. The platform history was rewritten on 2026-10-04 (split out of the data repo): that sha
    # no longer resolves anywhere. 7d2ae3a ("Plataforma S3: supervision", 2026-10-03) is the closest old commit by
    # slice naming: an INFERENCE of the engine team, not a statement of the product team.
    "unreachable_commits": {"a492bfa": "unreachable since 2026-10-04 (history rewritten); closest old match 7d2ae3a is an inference"},
}

DENIED_TABLES = ("login_accounts", "mfa_challenges", "staff_sessions")

# Columns that exist in the real platform but are never read (credentials, names, free-text
# derivatives, demo text). Kept explicit so the denial is testable and visible.
DENIED_COLUMNS = {
    "cases": ("search_text", "last_message_preview", "last_turn_preview", "rating_comment", "rating_key"),
    "turns": ("subject",),
    "customers": ("display_name", "suggestions"),
    "staff_dimensions": ("name", "email"),
}

ID_PATTERNS = {
    "case": r"^CASE-.+$",
    "turn": r"^TRN-.+$",
    "assignment": r"^ASG-.+$",
    "customer": r"^CUS-.+$",
    "staff": r"^STF-.+$",
    # 1.2.0: an assistant turn's author_id is the agent `id@version` (ADR 0003)
    "actor": r"^((CUS|STF)-.+|[A-Za-z0-9_.-]+@[A-Za-z0-9_.^~-]+)$",
    "event": r"^EVT-.+$",
}

# 1.3.0: `cases.case_type` (platform `domain/cases/values.py::CaseType`; `virtual_card` is team-generated).
CASE_TYPES = ["none", "unrecognized_charge", "undue_charge", "app_issue", "branch_service", "service_quality",
              "virtual_card"]

# 1.2.0: `assistant` (agent-core, ADR 0003) is an actor and an author role.
ROLES = ["customer", "analyst", "supervisor", "admin", "system", "assistant"]


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
            # 1.2.0: the platform renamed chat channels (chat_app/chat_web) and added phone/email; the 1.1.0 names stay readable.
            _c("channel", "string", enum=["app_chat", "web_chat", "chat_app", "chat_web", "phone_inbound",
                                          "phone_outbound", "email"]),
            _c("language", "string", enum=["es", "pt"]),
            _c("priority", "string", enum=["none", "low", "medium", "high", "critical"]),
            _c("status", "string", enum=["queued", "assigned", "in_progress", "closed", "with_assistant"]),
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
            # 1.2.0 (additive, optional so 1.1.0 rows stay valid). rating_comment / rating_key are never read.
            _c("rating_score", "integer", True, minimum=1, maximum=4),
            _c("rated_at", "datetime", True),
            _c("open_escalation_id", "string", True),
            _c("active_call_id", "string", True),
            # 1.3.0 (additive, optional so 1.2.0 rows stay valid): what the case is about (platform slice 18, ADR 0006).
            # A closed enum of dataset complaint subcategories + `none`; the platform column is NOT NULL.
            _c("case_type", "string", enum=CASE_TYPES),
            _c("version", "integer", minimum=1),
        ],
        "optional": ["rating_score", "rated_at", "open_escalation_id", "active_call_id", "case_type"],
    },
    "turns": {
        "source_table": "turns",
        "mutable": False,
        "primary_key": ["id"],
        "columns": [
            _c("id", "string", pattern=ID_PATTERNS["turn"]),
            _c("case_id", "string", pattern=ID_PATTERNS["case"]),
            _c("sequence", "integer", minimum=1),
            _c("kind", "string", enum=["message", "routing", "notice", "transcript", "note", "email"]),
            _c("audience", "string", enum=["everyone", "staff"]),
            _c("author_role", "string", enum=["customer", "analyst", "system", "assistant"]),
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
            _c("reason", "string", enum=["language_least_loaded", "queue_drained", "manual", "outbound_call",
                                       "assistant_handoff"]),
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
EVENT_CATALOG_VERSION = "1.3.0"
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
    # --- 1.2.0 (platform eeb73a8: application/audit/catalog.py and the domain events) ---
    ("case.priority_changed", "cases", "case", "admitted"),
    ("case.rated", "cases", "case", "admitted"),
    ("case.assistant_started", "cases", "case", "admitted"),
    ("case.assistant_released", "assignment", "case", "admitted"),
    ("assistant.session_started", "assistant", "assistant", "admitted"),
    ("assistant.input_queued", "assistant", "assistant", "admitted"),
    ("assistant.turn_answered", "assistant", "assistant", "admitted"),
    ("assistant.step_up_verified", "assistant", "assistant", "admitted"),
    ("assistant.step_up_rejected", "assistant", "assistant", "admitted"),
    ("assistant.ended", "assistant", "assistant", "admitted"),
    ("copilot.query_asked", "copilot", "copilot", "admitted"),
    ("copilot.answered", "copilot", "copilot", "admitted"),
    ("builder.proposal_created", "agents", "builder", "admitted"),
    ("builder.proposal_tracked", "agents", "builder", "admitted"),
    ("builder.draft_saved", "agents", "builder", "admitted"),
    ("builder.proposal_validated", "agents", "builder", "admitted"),
    ("builder.proposal_frozen", "agents", "builder", "admitted"),
    ("builder.proposal_reopened", "agents", "builder", "admitted"),
    ("builder.proposal_evaluated", "agents", "builder", "admitted"),
    ("builder.proposal_approved", "agents", "builder", "admitted"),
    ("builder.proposal_rejected", "agents", "builder", "admitted"),
    ("builder.proposal_published", "agents", "builder", "admitted"),
    ("builder.alias_promoted", "agents", "builder", "admitted"),
    ("builder.release_revoked", "agents", "builder", "admitted"),
    ("builder.question_asked", "agents", "builder", "admitted"),
    ("builder.answered", "agents", "builder", "admitted"),
    ("escalation.opened", "escalation", "escalation", "admitted"),
    ("escalation.withdrawn", "escalation", "escalation", "admitted"),
    ("escalation.answered", "escalation", "escalation", "admitted"),
    ("escalation.taken", "escalation", "escalation", "admitted"),
    ("escalation.reassigned", "escalation", "escalation", "admitted"),
    ("escalation.closed", "escalation", "escalation", "admitted"),
    ("escalation.acknowledged", "escalation", "escalation", "admitted"),
    ("call.started", "conversation", "call", "admitted"),
    ("call.answered", "conversation", "call", "admitted"),
    ("call.held", "conversation", "call", "admitted"),
    ("call.resumed", "conversation", "call", "admitted"),
    ("call.mute_changed", "conversation", "call", "admitted"),
    ("call.ended", "conversation", "call", "admitted"),
    # --- 1.3.0 (platform 5261ecf): copilot suggestions (ADR 0005), tool feedback, case type, AI maturity, AI switch ---
    ("copilot.suggestion_requested", "copilot", "copilot", "admitted"),
    ("copilot.suggestion_ready", "copilot", "copilot", "admitted"),
    ("copilot.suggestion_none", "copilot", "copilot", "admitted"),
    ("copilot.suggestion_failed", "copilot", "copilot", "admitted"),
    ("copilot.suggestion_decided", "copilot", "copilot", "admitted"),
    ("copilot.tool_used", "copilot", "copilot", "admitted"),
    ("case.type_changed", "cases", "case", "admitted"),
    ("ai.stage_advanced", "maturity", "case_type", "admitted"),
    ("ai.stage_moved_back", "maturity", "case_type", "admitted"),
    ("ai.agent_ready", "maturity", "case_type", "admitted"),
    ("ai.agent_activated", "maturity", "case_type", "admitted"),
    ("platform.ai_toggled", "platform", "platform", "admitted"),
]

# Data class of every admitted type (what its payload may carry). The platform states in its domain events that the
# AI payloads carry "ids, enums and counters only: never message text, never a confirmation's summary, never a
# credential" (domain/ai/events.py). Free text that the platform DOES put in a few operational payloads is listed in
# EVENT_FREE_TEXT_KEYS: the exporter drops those keys for that type (fail closed, even when the key name would not
# trip the generic redaction tokens).
EVENT_DATA_CLASSES = {
    "operational": "case workflow facts: ids, enums, counters, timestamps (free-text keys listed per type are dropped)",
    "assistant": "agent-core assistant runtime metadata: session/run/trace ids, agent id@version, enums, counters",
    "copilot": "analyst copilot metadata: question id and size, agent id, run/trace ids, status, counters (no text)",
    "builder": "agent-builder audit: proposal/agent/release ids, alias, hashes, verdict enums, counters (no draft content)",
    "maturity": "AI maturity per case type and the platform AI switch: case type enum, stage numbers, agent id, flags "
                "(1.3.0; no text, no case id)",
}
_ASSISTANT_TYPES = frozenset({
    "assistant.session_started", "assistant.input_queued", "assistant.turn_answered", "assistant.step_up_verified",
    "assistant.step_up_rejected", "assistant.ended", "case.assistant_started", "case.assistant_released"})


def event_data_class(event_type: str) -> str | None:
    """Data class of an admitted event type; None for denied/planned/unknown types."""
    for t, _, _, s in EVENT_TYPES:
        if t == event_type:
            if s != "admitted":
                return None
            if event_type in _ASSISTANT_TYPES:
                return "assistant"
            return {"copilot": "copilot", "builder": "builder", "ai": "maturity", "platform": "maturity"}.get(
                event_type.split(".", 1)[0], "operational")
    return None


# Payload keys per 1.2.0 type, from the platform's domain events (domain/cases/events.py, domain/ai/events.py).
# `entity`, `entity_id` and `case_id` live on the event_log row, not in the payload.
EVENT_PAYLOAD_KEYS = {
    "case.priority_changed": ("from_priority", "to_priority"),
    "case.rated": ("score", "comment", "analyst_id"),
    "case.assistant_started": ("assistant_session_id", "agent"),
    "case.assistant_released": ("reason", "handoff_ref", "sla_due_at"),
    "assistant.session_started": ("customer_id", "agent"),
    "assistant.input_queued": ("kind", "answer"),
    # 1.3.0: + release (the registry release id the run started on; absent on rows written before platform 2d868a2)
    "assistant.turn_answered": ("agent", "run_id", "awaiting", "status", "outcome", "trace_id", "messages", "release"),
    "assistant.step_up_verified": ("simulated",),
    "assistant.step_up_rejected": ("attempts",),
    "assistant.ended": ("result", "handoff_ref", "code"),
    "copilot.query_asked": ("question_id", "question_length"),
    "copilot.answered": ("question_id", "agent", "run_id", "trace_id", "status", "messages"),
    "builder.proposal_created": ("agent_id", "origin", "base_release_id"),
    "builder.proposal_tracked": ("agent_id", "source"),
    "builder.draft_saved": ("agent_id", "rev", "changes", "kinds"),
    "builder.proposal_validated": ("agent_id", "violations", "candidate_hash"),
    "builder.proposal_frozen": ("agent_id", "candidate_hash", "new_versions"),
    "builder.proposal_reopened": ("agent_id", "rev"),
    "builder.proposal_evaluated": ("agent_id", "suite_id", "verdict", "items", "items_failed"),
    "builder.proposal_approved": ("agent_id", "candidate_hash", "yardstick_loosened", "step_up"),
    "builder.proposal_rejected": ("agent_id", "reason_length", "step_up"),
    "builder.proposal_published": ("agent_id", "release_id", "step_up"),
    "builder.alias_promoted": ("alias", "release_id", "before", "reason_length", "step_up"),
    "builder.release_revoked": ("agent_id", "reason_length", "step_up"),
    "builder.question_asked": ("question_id", "question_length"),
    "builder.answered": ("question_id", "agent", "run_id", "trace_id", "status", "messages"),
    "escalation.opened": ("motive", "analyst_id"),
    "escalation.withdrawn": ("analyst_id",),
    "escalation.answered": ("note", "analyst_id"),
    "escalation.taken": ("previous_analyst_id", "analyst_id"),
    "escalation.reassigned": ("previous_analyst_id", "analyst_id"),
    "escalation.closed": ("analyst_id",),
    "escalation.acknowledged": ("analyst_id",),
    "call.started": ("direction", "customer_id", "analyst_id", "reason"),
    "call.answered": ("answered_by_role", "analyst_id", "ring_seconds"),
    "call.held": ("analyst_id",),
    "call.resumed": ("analyst_id", "hold_seconds"),
    "call.mute_changed": ("muted", "analyst_id"),
    "call.ended": ("end_reason", "ended_by_role", "analyst_id", "answered", "duration_seconds", "hold_seconds"),
    # --- 1.3.0 (platform domain/ai/events.py, maturity_events.py, domain/cases/events.py, domain/platform/events.py).
    # `release`/`turn_id` may be absent on rows written before the platform carried them: null, never drift.
    "copilot.suggestion_requested": ("analyst_id", "trigger", "based_on_sequence"),
    "copilot.suggestion_ready": ("analyst_id", "agent", "kinds", "count", "truncated", "run_id", "trace_id",
                                 "release"),
    "copilot.suggestion_none": ("analyst_id", "agent", "run_id", "trace_id", "release"),
    "copilot.suggestion_failed": ("analyst_id", "failure_code"),
    "copilot.suggestion_decided": ("subject", "decision", "edit_distance_permille", "turn_id", "agent", "release"),
    "copilot.tool_used": ("tool",),
    "case.type_changed": ("from", "to"),
    "ai.stage_advanced": ("case_type", "from_stage", "to_stage"),
    "ai.stage_moved_back": ("case_type", "from_stage", "to_stage", "agent_cleared"),
    "ai.agent_ready": ("case_type",),
    "ai.agent_activated": ("case_type", "agent_id"),
    "platform.ai_toggled": ("enabled",),
}
# Closed value sets (1.3.0). A key listed here is forwarded only while its value is in the set (the exporter redacts
# it otherwise): `subject` looks like free text by name but is the two-value enum reply|escalation on this type.
EVENT_PAYLOAD_ENUMS = {
    "copilot.suggestion_requested": {"trigger": ("customer_message", "manual", "handover")},
    "copilot.suggestion_decided": {"subject": ("reply", "escalation"),
                                   "decision": ("used", "edited", "discarded", "ignored", "accepted")},
}
# Keys that carry free text (a customer's words, an analyst's note, a motive). Never forwarded for that type.
EVENT_FREE_TEXT_KEYS = {
    "case.rated": ("comment",),
    "assistant.input_queued": ("answer",),
    "escalation.opened": ("motive",),
    "escalation.answered": ("note",),
    "call.started": ("reason",),
    # older admitted types whose payload the platform also fills with text
    "case.closed": ("note",),
    # 1.3.0: staff_line = facts of a staff-only line; its params hold staff names "as they were then" (slice 23c)
    "turn.created": ("text", "subject", "staff_line"),
}

# Announced by Product (administration slice, uncommitted at capture): staff.* and team.*.
PLANNED_EVENT_PREFIXES = ("staff.", "team.")
PLANNED_TABLES = ("teams", "admin_roster")

SQL_TYPES = {
    "string": "TEXT", "integer": "INTEGER", "boolean": "INTEGER",
    "datetime": "TEXT", "array": "TEXT", "object": "TEXT",
}


# --- Revision 1.1.0 (additive): exporter metadata gets its own discriminator. -----------------------------------
# 1.0.0 identified exporter metadata by the `exporter.` event_type prefix inside `source_event`. That rule stays valid
# as the documented interim for 1.0.0 (and for exporters configured with legacy_prefix=True). From 1.1.0 a
# `source_event` carries `kind`: `exporter_finding` (exporter metadata, never a domain row) or `domain_event`.
LEGACY_EXPORTER_PREFIX = "exporter."
SOURCE_EVENT_KINDS = {
    "domain_event": "row of the platform event_log, mapped to a domain observation",
    "exporter_finding": "exporter-authored metadata (quality/coverage/profile): never a domain row, never part of "
                        "source-sequence continuity",
}
# Closed list: a code outside it is unsupported and quarantined by the consumer.
FINDING_CODES = (
    "bad_row", "denied_event_type", "unknown_event_type", "late_event", "gap_suspected", "turn_sequence_gap",
    "capability_profile", "dimension_snapshot",
)
FINDING_SEVERITIES = ("info", "warning", "error")
FINDING_IDENTITY_PATTERN = r"^(finding|profile|dimensions):.+$"
MAX_FINDING_DETAILS_BYTES = 32768  # serialized bound on `details` (checked by conformance, not JSON Schema)
MAX_FINDING_DETAILS_PROPERTIES = 64
EVIDENCE_KINDS = ["observed", "team_generated"]
