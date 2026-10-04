"""Single source of truth for the Postgres DDL (schemas raw, augmented, product) and the loader allow-list.

`db/gen_ddl.py` renders `db/sql/*.sql` from this module; `db/tests` proves the committed SQL equals the render and
that the product allow-list mirrors `platform-exporter` (no drift). Only names and types live here, never data."""

from __future__ import annotations

import json
import re
from pathlib import Path

LINEAGE = (("_batch_id", "text"), ("_source_file", "text"), ("_ingested_at", "timestamptz"))
LINEAGE_NAMES = tuple(c for c, _ in LINEAGE)

# Evaluator tables and the pseudonym map are NEVER created, loaded or granted. Credential tables of the platform too.
FORBIDDEN_TABLES = frozenset({"labels", "timeline", "pseudonym_map", "login_accounts", "mfa_challenges",
                              "staff_sessions", "password_resets", "invitations", "dev_mailbox", "admin_roster"})

_T, _TS, _J, _I, _F, _B = "text", "timestamptz", "jsonb", "integer", "double precision", "boolean"

# Original E0 (`pulso_muestra_e0`, platform_history.json v0.5.1), 9 loadable tables. List/object columns are jsonb.
RAW_E0: dict[str, list[tuple[str, str]]] = {
    "e0_case": [("case_id", _T), ("customer_id", _T), ("opened_at", _TS), ("channel", _T), ("language", _T),
                ("origin", _T), ("topic", _T), ("complaint_id", _T), ("priority", _T), ("sla_due_at", _TS),
                ("assigned_analyst_id", _T)],
    "e0_turn": [("turn_id", _T), ("case_id", _T), ("event_time", _TS), ("author_role", _T), ("author_id", _T),
                ("text", _T), ("language", _T), ("from_suggestion_id", _T), ("evidence_ids", _J),
                ("text_source", _T)],
    "e0_identity_check": [("check_id", _T), ("case_id", _T), ("started_at", _TS), ("ended_at", _TS),
                          ("actor_role", _T), ("actor_id", _T), ("channel_session", _T), ("questions", _J),
                          ("questions_version", _T), ("correct", _I), ("result", _T), ("attempt", _I),
                          ("policy_rule_id", _T), ("trigger", _T)],
    "e0_routing_step": [("step_id", _T), ("case_id", _T), ("event_time", _TS), ("tier", _T), ("component_id", _T),
                        ("component_version", _T), ("outcome", _T), ("reason_code", _T), ("policy_rule_id", _T),
                        ("confidence", _F), ("inputs_used", _J), ("handoff", _J)],
    "e0_copilot_query": [("query_id", _T), ("case_id", _T), ("analyst_id", _T), ("event_time", _TS),
                         ("question_text", _T), ("query_signature", _T), ("tables_read", _J), ("columns_read", _J),
                         ("answered_by", _T), ("answer", _T), ("sent_to_chat", _B)],
    "e0_tool_call": [("call_id", _T), ("case_id", _T), ("event_time", _TS), ("actor_role", _T), ("actor_id", _T),
                     ("tool_id", _T), ("tool_version", _T), ("params", _J), ("permission_level", _T),
                     ("confirmed_by", _T), ("status", _T), ("verified", _B), ("state_change", _J),
                     ("retry_count", _I), ("latency_ms", _I), ("approval_id", _T)],
    "e0_approval": [("approval_id", _T), ("case_id", _T), ("requested_at", _TS), ("requested_by_role", _T),
                    ("requested_by_id", _T), ("tool_id", _T), ("params", _J), ("reason_code", _T),
                    ("policy_rule_id", _T), ("requester_note", _T), ("decided_by", _T), ("decided_at", _TS),
                    ("decision", _T), ("decision_note", _T), ("executed_call_id", _T)],
    "e0_case_close": [("case_id", _T), ("closed_at", _TS), ("closed_by_role", _T), ("resolved", _B),
                      ("contact_reason", _T), ("resolution_code", _T), ("followup_at", _TS), ("csat", _I)],
    "e0_signal": [("signal_id", _T), ("kind", _T), ("scope", _J), ("window_start", _TS), ("window_end", _TS),
                  ("support_cases", _I), ("support_analysts", _I), ("consistency", _T),
                  ("evidence_case_ids", _J), ("status", _T)],
}

_BANK = json.loads((Path(__file__).parent / "catalog" / "bank_tables.json").read_text(encoding="utf-8"))
RAW_BANK: dict[str, list[tuple[str, str]]] = {f"bank_{t}": [tuple(c) for c in cols] for t, cols in _BANK.items()}

# Canonical layer (dbt `canonical` models): same shape as E0 plus provenance of the source system. `cases` also
# carries the canonical customer_id and the pseudonym kept as source_customer_id.
_SS = [("source_system", _T)]
AUGMENTED: dict[str, list[tuple[str, str]]] = {
    "cases": [(c, t) for c, t in RAW_E0["e0_case"]] + [("source_customer_id", _T), ("language_source", _T)] + _SS,
    "case_closes": RAW_E0["e0_case_close"] + [("csat_raw", _I)] + _SS,
    "turns": RAW_E0["e0_turn"] + _SS,
    "identity_checks": RAW_E0["e0_identity_check"] + _SS,
    "routing_steps": RAW_E0["e0_routing_step"] + _SS,
    "copilot_queries": RAW_E0["e0_copilot_query"] + _SS,
    "tool_calls": RAW_E0["e0_tool_call"] + _SS,
    "approvals": RAW_E0["e0_approval"] + _SS,
    "signals": RAW_E0["e0_signal"] + _SS,
}

# Product (support-platform, contract 1.1.0): ONLY the columns of the exporter allow-list exist here. Mutable state,
# names, emails, message text and credential tables are absent by construction (denylist). Types follow tables.py.
PRODUCT: dict[str, list[tuple[str, str]]] = {
    "event_log": [("sequence", "bigint"), ("event_id", _T), ("event_type", _T), ("entity", _T), ("entity_id", _T),
                  ("case_id", _T), ("actor_role", _T), ("actor_id", _T), ("event_time", _TS),
                  ("ingested_at", _TS), ("payload", _J), ("tenant_id", _T)],
    "cases": [("id", _T), ("customer_id", _T), ("channel", _T), ("language", _T), ("priority", _T),
              ("opened_at", _TS), ("sla_due_at", _TS), ("previous_case_id", _T), ("tenant_id", _T)],
    "customers": [("id", _T), ("simulator", _B)],
    "staff": [("id", _T), ("roles", _J), ("languages", _J), ("team", _T), ("team_id", _T), ("active", _B)],
    "turns": [("id", _T), ("case_id", _T), ("sequence", _I), ("kind", _T), ("audience", _T), ("author_role", _T),
              ("created_at", _TS)],
    "assignments": [("id", _T), ("case_id", _T), ("staff_id", _T), ("reason", _T), ("policy_rule_id", _T),
                    ("strategy", _T), ("open_cases_at_assignment", _I), ("waited_seconds", _I),
                    ("previous_staff_id", _T), ("paused_override", _B), ("assigned_at", _TS)],
    "customer_case_slots": [("customer_id", _T), ("open_case_id", _T)],
}

SCHEMAS: dict[str, dict[str, list[tuple[str, str]]]] = {
    "raw": {**RAW_E0, **RAW_BANK}, "augmented": AUGMENTED, "product": PRODUCT}

# Read-only role per schema (NOLOGIN here; bootstrap sets LOGIN + password from secrets, never in SQL).
READER_ROLE = {"raw": "pulso_raw_ro", "augmented": "pulso_augmented_ro", "product": "pulso_product_ro"}
LOADER_ROLE = "pulso_loader"
APP_ROLE = "pulso_app"

_SENSITIVE_TOKENS = frozenset({"text", "email", "phone", "address", "note", "notes", "comment", "comments",
                               "description", "transcript", "answer", "name", "document", "birth", "postal", "ip",
                               "params", "referrer"})
_SENSITIVE_EXTRA = frozenset({("bank_complaints", "resolution")})
_SPLIT = re.compile(r"[^A-Za-z0-9]+")


def is_sensitive(table: str, column: str) -> bool:
    """Free text, names, contact data and identifiers: not granted to readers (mirrors the platform denylist).
    Product columns are already allow-listed, so nothing there is sensitive."""
    if table in PRODUCT:
        return False
    if (table, column) in _SENSITIVE_EXTRA:
        return True
    toks = [t for t in _SPLIT.split(column.lower()) if t]
    return any(t in _SENSITIVE_TOKENS for t in toks)


def columns(schema: str, table: str) -> list[tuple[str, str]]:
    return SCHEMAS[schema][table]
