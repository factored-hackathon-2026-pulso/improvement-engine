"""Capability profile manifest (spec 32.2 item 6): what the observed platform has, versioned by phase.

Detectors derive denominators from it and answer `unsupported` / `insufficient_*` naming the missing capability. The
profile is introspected from the live schema (names only), so a superset schema (0.5.1 shape) is reported as such."""

from __future__ import annotations

from typing import Any

from .catalog import CATALOG_VERSION, KNOWN_EVENT_TYPES, SOURCE_NAMESPACE, jcs_digest
from .policy import ALLOWED_TABLES, DENIED_TABLES
from .source import SqlSource

PROFILE_VERSION = "platform_live.phase1/1"

# capability -> where it would show up: ("table", name) or ("column", table, column)
CAPABILITIES: dict[str, tuple[str, ...]] = {
    "origin": ("column", "cases", "origin"),
    "topic": ("column", "cases", "topic"),
    "complaint_id": ("column", "cases", "complaint_id"),
    "identity_check": ("table", "identity_check"),
    "routing_step": ("table", "routing_step"),
    "copilot_query": ("table", "copilot_query"),
    "tool_call": ("table", "tool_call"),
    "approval": ("table", "approval"),
    "suggestion": ("table", "suggestion"),
    "case_close.resolved": ("column", "cases", "resolved"),
    "csat": ("column", "cases", "csat"),
    "teams": ("table", "teams"),
}


def _columns_any(source: SqlSource, table: str) -> set[str]:
    """Column names of capability probes outside the read allow-list come from `table_columns` only for allowed tables;
    other tables are probed by presence only."""
    try:
        return set(source.table_columns(table))
    except PermissionError:
        return set()


def build_profile(source: SqlSource, extra_event_types: frozenset[str] = frozenset()) -> dict[str, Any]:
    tables = source.table_names()
    caps: dict[str, dict[str, str]] = {}
    for name, probe in CAPABILITIES.items():
        if probe[0] == "table":
            present = probe[1] in tables
        else:
            present = probe[2] in _probe_columns(source, probe[1], probe[2])
        caps[name] = {"status": "present" if present else "absent"}
    profile: dict[str, Any] = {
        "profile_version": PROFILE_VERSION,
        "phase": 1,
        "source_namespace": SOURCE_NAMESPACE,
        "catalog_version": CATALOG_VERSION,
        "capabilities": caps,
        "channels_declared": ["app_chat", "web_chat"],
        "tables_readable": sorted(t for t in ALLOWED_TABLES if t in tables),
        "tables_denied_by_construction": sorted(DENIED_TABLES),
        "event_types": sorted(KNOWN_EVENT_TYPES | extra_event_types),
        "unexpected_columns": source.unexpected_columns(),
    }
    profile["digest"] = jcs_digest({k: v for k, v in profile.items() if k != "digest"})
    return profile


def _probe_columns(source: SqlSource, table: str, column: str) -> set[str]:
    # `origin`/`topic`/`complaint_id` are not readable columns; their *existence* is a schema fact. Allowed tables expose
    # it through table_columns (names only, no row data).
    return _columns_any(source, table)
