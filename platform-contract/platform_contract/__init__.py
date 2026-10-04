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
    EVENT_DATA_CLASSES,
    EVENT_FREE_TEXT_KEYS,
    EVENT_PAYLOAD_KEYS,
    EVENT_TYPES,
    EVIDENCE_KINDS,
    FINDING_CODES,
    FINDING_IDENTITY_PATTERN,
    FINDING_SEVERITIES,
    ID_PATTERNS,
    LEGACY_EXPORTER_PREFIX,
    MAX_FINDING_DETAILS_BYTES,
    MAX_FINDING_DETAILS_PROPERTIES,
    PREVIOUS_CONTRACT_VERSION,
    SOURCE_EVENT_KINDS,
    PLANNED_EVENT_PREFIXES,
    PLANNED_TABLES,
    PROFILE,
    TABLES,
    event_data_class,
)

ROOT = Path(__file__).resolve().parents[1]
ALLOWED_TABLES = tuple(TABLES)
ADMITTED_EVENT_TYPES = tuple(t for t, _, _, s in EVENT_TYPES if s == "admitted")
# The platform stores UTC timestamps; offsets other than Z / +00:00 are contract violations.
UTC_DATETIME_PATTERN = r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|\+00:00)$"
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
        s: dict = {"type": "string", "format": "date-time", "pattern": UTC_DATETIME_PATTERN}
    elif kind == "array":
        s = {"type": "array"}
        if "items" in extra:
            s["items"] = extra["items"]
    else:
        s = {"type": kind}
    for k, v in extra.items():
        if k in ("pattern", "enum", "minimum", "maximum", "maxLength", "minLength"):
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


def _dt():
    return {"type": "string", "format": "date-time", "pattern": UTC_DATETIME_PATTERN}


def _source_event_schema(name: str) -> dict:
    """Body schema of `source_event` for one kind (1.1.0)."""
    common = {
        "source_namespace": {"const": "platform_live"},
        "catalog_version": {"type": "string", "minLength": 1},
        "tenant_id": {"type": "string", "minLength": 1},
    }
    if name == "exporter_finding":
        props = {
            "kind": {"const": "exporter_finding"}, **common,
            "finding_code": {"type": "string", "enum": list(FINDING_CODES)},
            "severity": {"type": "string", "enum": list(FINDING_SEVERITIES)},
            "described_native_event_id": {"type": ["string", "null"]},
            "described_source_sequence": {"type": ["integer", "null"], "minimum": 0},
            "details": {"type": "object", "maxProperties": MAX_FINDING_DETAILS_PROPERTIES},
        }
        desc = ("Exporter metadata (quality/coverage/profile). Identity (tenant_id, source_id, native_event_id) and "
                "observed_at live on the observation envelope, outside the digest; never a domain row and never part of source-sequence continuity. `details` is inline and bounded "
                f"(<= {MAX_FINDING_DETAILS_BYTES} serialized bytes, checked by conformance).")
    else:
        props = {
            "kind": {"const": "domain_event"}, **common,
            "event_type": {"type": "string", "minLength": 1,
                           "not": {"pattern": "^" + LEGACY_EXPORTER_PREFIX.replace(".", "\\.")}},
            "event_id": {"type": "string", "pattern": ID_PATTERNS["event"]},
            "event_time": _dt(), "ingested_at": _dt(), "available_at": _dt(),
            "case_id": {"type": ["string", "null"]},
            "entity": {"type": ["string", "null"]}, "entity_id": {"type": ["string", "null"]},
            "actor_role": {"type": ["string", "null"]}, "actor_ref": {"type": ["string", "null"]},
            "payload": {"type": "object"},
            "redacted_fields": {"type": "array", "items": {"type": "string"}},
            "evidence_kind": {"type": "string", "enum": EVIDENCE_KINDS},
            "population_excluded": {"type": "boolean"},
            "late": {"type": "boolean"},
        }
        desc = "Domain row of the platform event_log (event_type must not use the legacy `exporter.` prefix)."
    return {
        "$schema": SCHEMA_DIALECT,
        "$id": f"platform_live/{CONTRACT_VERSION}/{name}.schema.json",
        "title": name,
        "description": desc,
        "type": "object",
        "properties": props,
        "required": list(props),
        "additionalProperties": False,
        "x-contract-version": CONTRACT_VERSION,
    }


def build_source_schema(name: str) -> dict:
    if name in ("exporter_finding", "domain_event"):
        return _source_event_schema(name)
    if name != "source_observation":
        raise KeyError(name)
    defs = {}
    for k in ("exporter_finding", "domain_event"):
        d = _source_event_schema(k)
        for drop in ("$schema", "$id", "x-contract-version"):
            d.pop(drop)
        defs[k] = d
    return {
        "$schema": SCHEMA_DIALECT,
        "$id": f"platform_live/{CONTRACT_VERSION}/source_observation.schema.json",
        "title": "source_observation",
        "description": ("Self-contained view of one `platform_event` observation: identity "
                        "(tenant_id, source_id, native_event_id), nullable source_sequence and the discriminated "
                        "`source_event` (`kind`). source_sequence is null for findings not tied to a row."),
        "type": "object",
        "properties": {
            "tenant_id": {"type": "string", "minLength": 1},
            "source_id": {"type": "string", "minLength": 1},
            "native_event_id": {"type": "string", "minLength": 1},
            "source_sequence": {"type": ["integer", "null"], "minimum": 0},
            "observed_at": _dt(),
            "source_event": {"oneOf": [{"$ref": "#/$defs/exporter_finding"}, {"$ref": "#/$defs/domain_event"}]},
        },
        "required": ["tenant_id", "source_id", "native_event_id", "source_sequence", "observed_at", "source_event"],
        "additionalProperties": False,
        "allOf": [{
            "if": {"properties": {"source_event": {"properties": {"kind": {"const": "exporter_finding"}},
                                                    "required": ["kind"]}}},
            "then": {"properties": {"native_event_id": {"pattern": FINDING_IDENTITY_PATTERN}}},
        }],
        "$defs": defs,
        "x-contract-version": CONTRACT_VERSION,
    }


def _event_entry(t: str, f: str, e: str, s: str) -> dict:
    entry = {"event_type": t, "family": f, "entity": e, "status": s}
    if s == "admitted":
        entry["data_class"] = event_data_class(t)
    if t in EVENT_PAYLOAD_KEYS:
        entry["payload_keys"] = list(EVENT_PAYLOAD_KEYS[t])
    if t in EVENT_FREE_TEXT_KEYS:
        entry["free_text_keys"] = list(EVENT_FREE_TEXT_KEYS[t])
    return entry


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
        "source_event_kinds": dict(SOURCE_EVENT_KINDS),
        "exporter_finding": {
            "finding_codes": list(FINDING_CODES),
            "severities": list(FINDING_SEVERITIES),
            "schema": "schemas/exporter_finding.schema.json",
            "identity": "(tenant_id, source_id, native_event_id)",
            "excluded_from": ["domain observations", "population/window projections", "source-sequence continuity"],
        },
        "legacy_exporter_prefix": {
            "prefix": LEGACY_EXPORTER_PREFIX,
            "valid_for_contract_versions": ["1.0.0"],
            "status": "interim rule of 1.0.0; under 1.1.0 only exporters with legacy_prefix=True still emit it",
        },
        "unknown_policy": "count, quarantine with a quality finding, never fail the batch",
        "planned_prefixes": list(PLANNED_EVENT_PREFIXES),
        "data_classes": dict(EVENT_DATA_CLASSES),
        "event_types": [_event_entry(t, f, e, s) for t, f, e, s in EVENT_TYPES],
    }


def classify_event_type(event_type: str) -> str:
    """admitted | denied | planned | unknown (anything but admitted is not ingested)."""
    for t, _, _, s in EVENT_TYPES:
        if t == event_type:
            return s
    if event_type.startswith(PLANNED_EVENT_PREFIXES):
        return "planned"
    return "unknown"


SOURCE_SCHEMAS = ("exporter_finding", "domain_event", "source_observation")


def _dump(obj) -> str:
    return json.dumps(obj, indent=2, ensure_ascii=False) + "\n"


def generate_artifacts() -> dict[str, str]:
    out = {f"schemas/{t}.schema.json": _dump(build_schema(t)) for t in TABLES}
    for k in SOURCE_SCHEMAS:
        out[f"schemas/{k}.schema.json"] = _dump(build_source_schema(k))
    out["event-catalog.json"] = _dump(build_event_catalog())
    return out


def load_source_schema(name: str) -> dict:
    if name not in SOURCE_SCHEMAS:
        raise KeyError(name)
    return json.loads((ROOT / "schemas" / f"{name}.schema.json").read_text(encoding="utf-8"))


def load_schema(table: str) -> dict:
    assert_table_readable(table)
    return json.loads((ROOT / "schemas" / f"{table}.schema.json").read_text(encoding="utf-8"))


__all__ = [
    "ADMITTED_EVENT_TYPES", "ALLOWED_TABLES", "ARTIFACT_STAMP", "CONTRACT_VERSION",
    "DENIED_COLUMNS", "DENIED_TABLES", "PLANNED_TABLES", "PROFILE", "TABLES", "TableRefused",
    "assert_table_readable", "build_event_catalog", "build_schema", "classify_event_type",
    "generate_artifacts", "load_schema", "load_source_schema", "build_source_schema", "SOURCE_SCHEMAS",
    "PREVIOUS_CONTRACT_VERSION", "MAX_FINDING_DETAILS_BYTES", "LEGACY_EXPORTER_PREFIX",
    "EVENT_DATA_CLASSES", "EVENT_FREE_TEXT_KEYS", "EVENT_PAYLOAD_KEYS", "event_data_class",
]
