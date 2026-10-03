"""platform_live contract pack: schemas, event catalog, allow-list/denylist, generator."""

from __future__ import annotations

import json
from pathlib import Path

from .model import (
    ARTIFACT_STAMP,
    CONTRACT_VERSION,
    DENIED_COLUMNS,
    DENIED_TABLES,
    EVENT_CATALOG_VERSION,
    EVENT_TYPES,
    PLANNED_EVENT_PREFIXES,
    PLANNED_TABLES,
    PROFILE,
    TABLES,
)

ROOT = Path(__file__).resolve().parents[1]
ALLOWED_TABLES = tuple(TABLES)
ADMITTED_EVENT_TYPES = tuple(t for t, _, _, s in EVENT_TYPES if s == "admitted")
SCHEMA_DIALECT = "https://json-schema.org/draft/2020-12/schema"


class TableRefused(Exception):
    """Raised before any read of a table that is denied or not allow-listed."""

    def __init__(self, table: str, category: str):
        super().__init__(f"table {table!r} refused: {category}")
        self.table = table
        self.category = category


def assert_table_readable(table: str) -> None:
    """Contract-level table name (`staff` is only readable as `staff_dimensions`)."""
    if table in DENIED_TABLES:
        raise TableRefused(table, "denied")
    if table not in TABLES:
        raise TableRefused(table, "not_allowlisted")


def _prop(kind, nullable, extra):
    if kind == "datetime":
        s: dict = {"type": "string", "format": "date-time"}
    elif kind == "array":
        s = {"type": "array"}
        if "items" in extra:
            s["items"] = extra["items"]
    else:
        s = {"type": kind}
    for k, v in extra.items():
        if k in ("pattern", "enum", "minimum", "maxLength", "minLength"):
            s[k] = v
    if extra.get("sensitive_text"):
        s["x-sensitive-text"] = True
    if nullable:
        s["type"] = [s["type"], "null"]
        if "enum" in s:
            s["enum"] = [*s["enum"], None]
    return s


def build_schema(table: str) -> dict:
    spec = TABLES[table]
    optional = set(spec.get("optional", ()))
    props = {n: _prop(k, nl, ex) for n, k, nl, ex in spec["columns"]}
    kind = "mutable, optimistic version" if spec["mutable"] else "append-only"
    return {
        "$schema": SCHEMA_DIALECT,
        "$id": f"platform_live/{CONTRACT_VERSION}/{table}.schema.json",
        "title": table,
        "description": f"platform_live row from source table `{spec['source_table']}` ({kind}).",
        "type": "object",
        "properties": props,
        "required": [n for n in props if n not in optional],
        "additionalProperties": False,
        "x-primary-key": spec["primary_key"],
        "x-denied-columns": list(DENIED_COLUMNS.get(table, ())),
        "x-contract-version": CONTRACT_VERSION,
    }


def build_event_catalog() -> dict:
    return {
        "catalog_version": EVENT_CATALOG_VERSION,
        "contract_version": CONTRACT_VERSION,
        "profile": PROFILE,
        "statuses": {
            "admitted": "ingest",
            "denied": "known security/credential telemetry, never ingested",
            "planned": "announced by Product, not yet admitted: quarantine like unknown",
        },
        "unknown_policy": "count, quarantine with a quality finding, never fail the batch",
        "planned_prefixes": list(PLANNED_EVENT_PREFIXES),
        "event_types": [
            {"event_type": t, "family": f, "entity": e, "status": s} for t, f, e, s in EVENT_TYPES
        ],
    }


def classify_event_type(event_type: str) -> str:
    """admitted | denied | planned | unknown (anything but admitted is not ingested)."""
    for t, _, _, s in EVENT_TYPES:
        if t == event_type:
            return s
    if event_type.startswith(PLANNED_EVENT_PREFIXES):
        return "planned"
    return "unknown"


def _dump(obj) -> str:
    return json.dumps(obj, indent=2, ensure_ascii=False) + "\n"


def generate_artifacts() -> dict[str, str]:
    out = {f"schemas/{t}.schema.json": _dump(build_schema(t)) for t in TABLES}
    out["event-catalog.json"] = _dump(build_event_catalog())
    return out


def load_schema(table: str) -> dict:
    assert_table_readable(table)
    return json.loads((ROOT / "schemas" / f"{table}.schema.json").read_text(encoding="utf-8"))


__all__ = [
    "ADMITTED_EVENT_TYPES", "ALLOWED_TABLES", "ARTIFACT_STAMP", "CONTRACT_VERSION",
    "DENIED_COLUMNS", "DENIED_TABLES", "PLANNED_TABLES", "PROFILE", "TABLES", "TableRefused",
    "assert_table_readable", "build_event_catalog", "build_schema", "classify_event_type",
    "generate_artifacts", "load_schema",
]
