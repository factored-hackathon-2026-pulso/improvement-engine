"""Dependency-free semantic checks for the layer-0 artifact envelope fixtures.

The JSON Schema remains the interoperability contract. This checker executes
the v1 invariants that are important before a Rust validator exists.
"""

from __future__ import annotations

import json
import hashlib
import argparse
import re
import sys
from pathlib import Path, PurePosixPath, PureWindowsPath
from typing import Any


UUID_V7 = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
)
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
KINDS = frozenset(
    {
        "signal",
        "opportunity",
        "proposal",
        "capability_bundle",
        "scenario_set",
        "evaluation",
        "memory_wiki",
        "detector",
        "run_config",
        "source_snapshot",
    }
)
ENVELOPE_FIELDS = frozenset(
    {"contract_version", "tenant_id", "artifact", "payload", "run_config_ref", "source_snapshot_ref", "parent_refs"}
)
ARTIFACT_FIELDS = frozenset({"id", "revision", "kind", "digest"})
ARTIFACT_REF_FIELDS = frozenset({"tenant_id", "id", "revision", "digest"})
SOURCE_REF_FIELDS = frozenset(
    {"tenant_id", "source_namespace", "world_ref", "snapshot_ref", "table", "source_pk", "file_digest", "observed_cutoff"}
)
SOURCE_SNAPSHOT_FIELDS = frozenset(
    {"contract_version", "tenant_id", "source_namespace", "world_ref", "observed_cutoff", "sources"}
)
SOURCE_SNAPSHOT_SOURCE_FIELDS = frozenset(
    {"table", "uri", "file_digest", "partition_inventory_digest", "header_digest", "row_count", "source_contract_ref"}
)
SOURCE_CONTRACT_FIELDS = frozenset(
    {"contract_version", "source_namespace", "source_kind", "table", "primary_key", "event_clock", "read_only", "access_policy", "columns"}
)
SOURCE_COLUMN_FIELDS = frozenset({"name", "logical_type", "nullable", "purpose", "classification"})
SPEC22_FAMILIES = (
    "source_snapshot",
    "platform_observation",
    "signal",
    "workflow_bridge",
    "change_spec",
    "tree",
    "scenario",
    "evaluation",
    "release_ack",
    "memory_wiki",
    "read_model",
)


def _safe_repo_relative_ref(reference: str) -> bool:
    windows_path = PureWindowsPath(reference)
    posix_path = PurePosixPath(reference)
    return not (
        windows_path.is_absolute()
        or posix_path.is_absolute()
        or bool(windows_path.drive)
        or ".." in windows_path.parts
        or ".." in posix_path.parts
    )


def _is_published_json_schema(reference: str) -> bool:
    if not reference.endswith(".schema.json") or not _safe_repo_relative_ref(reference):
        return False
    root = Path(__file__).resolve().parents[1]
    schema_path = (root / reference).resolve()
    if not schema_path.is_relative_to(root) or not schema_path.is_file():
        return False
    try:
        value = json.loads(schema_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return False
    return isinstance(value, dict) and isinstance(value.get("type"), str) and isinstance(value.get("title"), str)


def validate_spec22_inventory(families: object) -> list[str]:
    """Validate family inventory and referenced contract evidence, not payloads."""
    if not isinstance(families, list):
        return ["invalid_spec22_inventory"]
    names: list[str] = []
    errors: list[str] = []
    for family in families:
        if not isinstance(family, dict) or not isinstance(family.get("name"), str):
            errors.append("invalid_family_entry")
            continue
        name = family["name"]
        names.append(name)
        if name not in SPEC22_FAMILIES:
            errors.append(f"unknown_family:{name}")
        contract_refs = family.get("contract_refs")
        if not isinstance(contract_refs, list) or any(not isinstance(ref, str) for ref in contract_refs):
            errors.append(f"invalid_contract_refs:{name}")
        else:
            errors.extend(
                f"unsafe_contract_ref:{name}:{ref}"
                for ref in contract_refs
                if not _safe_repo_relative_ref(ref)
            )
            status = family.get("contract_status")
            schema_refs = [ref for ref in contract_refs if _is_published_json_schema(ref)]
            if status == "published" and not schema_refs:
                errors.append(f"published_schema_missing_ref:{name}")
            elif status == "missing" and contract_refs:
                errors.append(f"missing_contract_has_ref:{name}")

        fixture_refs = family.get("fixture_refs")
        if not isinstance(fixture_refs, list) or any(not isinstance(ref, str) for ref in fixture_refs):
            errors.append(f"invalid_fixture_refs:{name}")
        else:
            errors.extend(
                f"unsafe_fixture_ref:{name}:{ref}"
                for ref in fixture_refs
                if not _safe_repo_relative_ref(ref)
            )
        status = family.get("contract_status")
        if not isinstance(status, str) or status not in {"published", "partial", "missing"}:
            errors.append(f"invalid_contract_status:{name}")
    for name in SPEC22_FAMILIES:
        if names.count(name) == 0:
            errors.append(f"missing_family:{name}")
        elif names.count(name) > 1:
            errors.append(f"duplicate_family:{name}")
    return errors


def spec22_coverage_report() -> dict[str, Any]:
    """Return evidence-backed Spec-22 contract coverage; no fixture is fabricated."""
    manifest = _load(Path(__file__).parent / "fixtures" / "spec22-v1" / "coverage.json")
    families = manifest.get("families", []) if isinstance(manifest, dict) else []
    validation = validate_spec22_inventory(families)
    root = Path(__file__).resolve().parents[1]
    missing_refs: list[str] = []
    for family in families:
        if not isinstance(family, dict):
            continue
        refs: list[object] = []
        for field in ("contract_refs", "fixture_refs"):
            value = family.get(field, [])
            if isinstance(value, list):
                refs.extend(value)
        for ref in refs:
            if not isinstance(ref, str) or not _safe_repo_relative_ref(ref):
                continue
            resolved = (root / ref).resolve()
            if not resolved.is_relative_to(root.resolve()) or not resolved.is_file():
                family_name = family.get("name")
                display_name = family_name if isinstance(family_name, str) else "<invalid>"
                missing_refs.append(f"missing_reference:{display_name}:{ref}")
    missing_contracts = [
        family.get("name", "<invalid>")
        for family in families
        if isinstance(family, dict) and family.get("contract_status") != "published"
    ]
    return {
        "family_count": len(families),
        "missing_families": [error.removeprefix("missing_family:") for error in validation if error.startswith("missing_family:")],
        "missing_wire_contracts": missing_contracts,
        "errors": validation + missing_refs,
        "families": families,
    }


def validate_spec22_coverage(*, strict: bool = False) -> list[str]:
    """Strict mode is opt-in until all family wire contracts are published."""
    report = spec22_coverage_report()
    errors = list(report["errors"])
    if strict:
        errors.extend(f"wire_contract_not_published:{name}" for name in report["missing_wire_contracts"])
    return errors


def validate_envelope(value: object) -> list[str]:
    """Return stable validation codes; never echo fixture content."""
    if not isinstance(value, dict):
        return ["invalid_envelope"]

    errors: list[str] = []
    if set(value) - ENVELOPE_FIELDS:
        errors.append("unexpected_envelope_field")
    version = value.get("contract_version")
    if not isinstance(version, dict) or set(version) != {"major", "minor"} or version.get("major") != 1:
        errors.append("unsupported_contract_major")
    elif not isinstance(version.get("minor"), int) or isinstance(version["minor"], bool) or version["minor"] < 0:
        errors.append("invalid_contract_minor")

    tenant_id = value.get("tenant_id")
    if not isinstance(tenant_id, str) or not tenant_id or len(tenant_id) > 128:
        errors.append("invalid_tenant_id")

    artifact = value.get("artifact")
    if not isinstance(artifact, dict):
        errors.append("invalid_artifact")
    else:
        if set(artifact) - ARTIFACT_FIELDS:
            errors.append("invalid_artifact")
        artifact_id = artifact.get("id")
        if not isinstance(artifact_id, str) or not UUID_V7.fullmatch(artifact_id):
            errors.append("invalid_artifact_id")
        revision = artifact.get("revision")
        if not isinstance(revision, int) or isinstance(revision, bool) or revision < 1:
            errors.append("invalid_artifact_revision")
        if artifact.get("kind") not in KINDS:
            errors.append("invalid_artifact_kind")
        digest = artifact.get("digest")
        if not isinstance(digest, str) or not SHA256.fullmatch(digest):
            errors.append("invalid_artifact_digest")

    if not isinstance(value.get("payload"), dict):
        errors.append("invalid_payload")

    for ref_name in ("run_config_ref", "source_snapshot_ref"):
        if ref_name in value:
            errors.extend(_validate_ref(value[ref_name], tenant_id, ref_name))

    parent_refs = value.get("parent_refs", [])
    if not isinstance(parent_refs, list):
        errors.append("invalid_parent_refs")
    else:
        seen_refs: set[tuple[object, ...]] = set()
        for reference in parent_refs:
            errors.extend(_validate_ref(reference, tenant_id, "parent_ref"))
            if isinstance(reference, dict):
                identity = tuple(reference.get(field) for field in ("tenant_id", "id", "revision", "digest"))
                if identity in seen_refs:
                    errors.append("duplicate_parent_ref")
                seen_refs.add(identity)
    return errors


def _validate_ref(reference: object, tenant_id: object, _: str) -> list[str]:
    return _validate_artifact_ref(reference, tenant_id)


def _validate_artifact_ref(reference: object, tenant_id: object | None = None) -> list[str]:
    if not isinstance(reference, dict):
        return ["invalid_artifact_ref"]
    errors: list[str] = []
    if set(reference) - ARTIFACT_REF_FIELDS:
        return ["invalid_artifact_ref"]
    if tenant_id is not None and reference.get("tenant_id") != tenant_id:
        errors.append("cross_tenant_reference")
    if not isinstance(reference.get("tenant_id"), str) or not reference["tenant_id"]:
        errors.append("invalid_artifact_ref")
    ref_id = reference.get("id")
    if not isinstance(ref_id, str) or not UUID_V7.fullmatch(ref_id):
        errors.append("invalid_artifact_ref")
    revision = reference.get("revision")
    if not isinstance(revision, int) or isinstance(revision, bool) or revision < 1:
        errors.append("invalid_artifact_ref")
    digest = reference.get("digest")
    if not isinstance(digest, str) or not SHA256.fullmatch(digest):
        errors.append("invalid_artifact_ref")
    return errors


def _valid_version(value: object) -> bool:
    return (
        isinstance(value, dict)
        and set(value) == {"major", "minor"}
        and value.get("major") == 1
        and isinstance(value.get("minor"), int)
        and not isinstance(value["minor"], bool)
        and value["minor"] >= 0
    )


def _valid_timestamp(value: object) -> bool:
    return isinstance(value, str) and bool(re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", value))


def _valid_table_name(value: object) -> bool:
    return isinstance(value, str) and bool(re.fullmatch(r"[a-z][a-z0-9_]*", value))


def validate_source_ref(value: object) -> list[str]:
    if not isinstance(value, dict):
        return ["invalid_source_ref"]
    errors: list[str] = []
    if set(value) - SOURCE_REF_FIELDS:
        errors.append("invalid_source_ref")
    tenant_id = value.get("tenant_id")
    if not isinstance(tenant_id, str) or not tenant_id or len(tenant_id) > 128:
        errors.append("invalid_tenant_id")
    for field in ("source_namespace", "world_ref"):
        if not isinstance(value.get(field), str) or not value[field]:
            errors.append(f"invalid_{field}")
    if not _valid_table_name(value.get("table")):
        errors.append("invalid_table")
    if not isinstance(value.get("source_pk"), str) or not value["source_pk"]:
        errors.append("invalid_source_pk")
    digest = value.get("file_digest")
    if not isinstance(digest, str) or not SHA256.fullmatch(digest):
        errors.append("invalid_file_digest")
    if not _valid_timestamp(value.get("observed_cutoff")):
        errors.append("invalid_observed_cutoff")
    snapshot_ref = value.get("snapshot_ref")
    errors.extend(_validate_artifact_ref(snapshot_ref, tenant_id))
    return errors


def validate_source_snapshot(value: object) -> list[str]:
    if not isinstance(value, dict):
        return ["invalid_source_snapshot"]
    errors: list[str] = []
    if set(value) - SOURCE_SNAPSHOT_FIELDS:
        errors.append("invalid_source_snapshot")
    if not _valid_version(value.get("contract_version")):
        errors.append("unsupported_contract_major")
    tenant_id = value.get("tenant_id")
    if not isinstance(tenant_id, str) or not tenant_id or len(tenant_id) > 128:
        errors.append("invalid_tenant_id")
    for field in ("source_namespace", "world_ref"):
        if not isinstance(value.get(field), str) or not value[field]:
            errors.append(f"invalid_{field}")
    if not _valid_timestamp(value.get("observed_cutoff")):
        errors.append("invalid_observed_cutoff")
    sources = value.get("sources")
    if not isinstance(sources, list) or not sources:
        return errors + ["invalid_snapshot_sources"]
    for source in sources:
        if not isinstance(source, dict):
            errors.append("invalid_snapshot_source")
            continue
        if set(source) - SOURCE_SNAPSHOT_SOURCE_FIELDS:
            errors.append("invalid_snapshot_source")
        required = ("table", "uri", "file_digest", "header_digest", "row_count", "source_contract_ref")
        if any(key not in source for key in required):
            errors.append("invalid_snapshot_source")
            continue
        if not _valid_table_name(source["table"]):
            errors.append("invalid_snapshot_source")
        if not isinstance(source["uri"], str) or not source["uri"].startswith(("file://", "s3://")):
            errors.append("invalid_snapshot_source")
        for digest_name in ("file_digest", "header_digest"):
            if not isinstance(source[digest_name], str) or not SHA256.fullmatch(source[digest_name]):
                errors.append("invalid_snapshot_source")
        if "partition_inventory_digest" in source and (
            not isinstance(source["partition_inventory_digest"], str)
            or not SHA256.fullmatch(source["partition_inventory_digest"])
        ):
            errors.append("invalid_snapshot_source")
        if not isinstance(source["row_count"], int) or isinstance(source["row_count"], bool) or source["row_count"] < 0:
            errors.append("invalid_snapshot_source")
        contract_ref = source["source_contract_ref"]
        if not isinstance(contract_ref, dict) or set(contract_ref) != {"id", "version", "digest"}:
            errors.append("invalid_snapshot_source")
        elif (
            not isinstance(contract_ref["id"], str)
            or not re.fullmatch(r"[a-z][a-z0-9_.-]*", contract_ref["id"])
            or not isinstance(contract_ref["version"], str)
            or not re.fullmatch(r"v[1-9][0-9]*", contract_ref["version"])
            or not isinstance(contract_ref["digest"], str)
            or not SHA256.fullmatch(contract_ref["digest"])
        ):
            errors.append("invalid_snapshot_source")
    return errors


def validate_source_contract(value: object) -> list[str]:
    if not isinstance(value, dict):
        return ["invalid_source_contract"]
    errors: list[str] = []
    if set(value) - SOURCE_CONTRACT_FIELDS:
        errors.append("invalid_source_contract")
    if not _valid_version(value.get("contract_version")):
        errors.append("unsupported_contract_major")
    if value.get("source_kind") != "original" or value.get("read_only") is not True:
        errors.append("invalid_source_access_mode")
    access_policy = value.get("access_policy")
    if (
        not isinstance(access_policy, dict)
        or set(access_policy) != {"permitted_classifications"}
        or not isinstance(access_policy.get("permitted_classifications"), list)
        or not access_policy["permitted_classifications"]
        or len(access_policy["permitted_classifications"]) != len(set(access_policy["permitted_classifications"]))
        or any(item not in {"internal", "pseudonymized", "aggregated"} for item in access_policy["permitted_classifications"])
    ):
        errors.append("invalid_source_access_policy")
    for field in ("source_namespace", "event_clock"):
        if not isinstance(value.get(field), str) or not value[field]:
            errors.append(f"invalid_{field}")
    if not _valid_table_name(value.get("table")):
        errors.append("invalid_table")
    primary_key = value.get("primary_key")
    if (
        not isinstance(primary_key, list)
        or not primary_key
        or len(primary_key) != len(set(primary_key))
        or any(not isinstance(key, str) or not key for key in primary_key)
    ):
        errors.append("invalid_primary_key")
    columns = value.get("columns")
    if not isinstance(columns, list) or not columns:
        return errors + ["invalid_source_columns"]
    names: set[str] = set()
    allowed_types = {"text", "timestamp", "date", "bool", "decimal", "int"}
    allowed_purposes = {"identity", "join", "event_clock", "metric", "dimension", "quality"}
    allowed_classifications = {"internal", "pseudonymized", "aggregated", "restricted"}
    for column in columns:
        if not isinstance(column, dict) or not isinstance(column.get("name"), str) or not column["name"]:
            errors.append("invalid_source_column")
            continue
        if column["name"] in names:
            errors.append("duplicate_source_column")
        names.add(column["name"])
        if set(column) - SOURCE_COLUMN_FIELDS:
            errors.append("invalid_source_column")
        if (
            column.get("logical_type") not in allowed_types
            or column.get("purpose") not in allowed_purposes
            or column.get("classification") not in allowed_classifications
        ):
            errors.append("invalid_source_column")
        if not isinstance(column.get("nullable"), bool):
            errors.append("invalid_source_column")
    if isinstance(primary_key, list) and any(key not in names for key in primary_key):
        errors.append("primary_key_not_declared")
    if value.get("event_clock") not in names:
        errors.append("event_clock_not_declared")
    if isinstance(access_policy, dict) and isinstance(access_policy.get("permitted_classifications"), list):
        permitted = set(access_policy["permitted_classifications"])
        if any(
            isinstance(column, dict)
            and column.get("classification") not in permitted
            and column.get("classification") != "restricted"
            for column in columns
        ):
            errors.append("source_column_classification_not_permitted")
    return errors


def _load(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def _validate_golden_header(contract: object, header_path: Path) -> list[str]:
    if not isinstance(contract, dict) or not isinstance(contract.get("columns"), list):
        return ["invalid_source_contract"]
    expected = [column.get("name") for column in contract["columns"] if isinstance(column, dict)]
    actual = header_path.read_text(encoding="utf-8").rstrip("\r\n").split(",")
    return [] if actual == expected else ["golden_header_mismatch"]


def _sha256_ref(path: Path) -> str:
    return f"sha256:{hashlib.sha256(path.read_bytes()).hexdigest()}"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--strict-spec22", action="store_true", help="fail unless all 11 family wire contracts are published")
    args = parser.parse_args()
    fixture_root = Path(__file__).parent / "fixtures" / "artifact-envelope-v1"
    failures: list[str] = []
    for fixture in sorted(fixture_root.glob("*.json")):
        errors = validate_envelope(_load(fixture))
        expected_invalid = fixture.name.startswith("invalid-")
        if (not expected_invalid and errors) or (expected_invalid and not errors):
            failures.append(fixture.name)
    source_fixture_root = Path(__file__).parent / "fixtures" / "sources-v1"
    validators = {
        "valid-call-center-interactions.json": validate_source_contract,
        "valid-source-snapshot.json": validate_source_snapshot,
        "valid-source-ref.json": validate_source_ref,
        "invalid-source-ref-empty-pk.json": validate_source_ref,
        "invalid-source-contract-writable.json": validate_source_contract,
    }
    for fixture_name, validator in validators.items():
        errors = validator(_load(source_fixture_root / fixture_name))
        expected_invalid = fixture_name.startswith("invalid-")
        if (not expected_invalid and errors) or (expected_invalid and not errors):
            failures.append(fixture_name)
    implementation_contract = Path(__file__).parent / "sources" / "call_center_interactions.v1.json"
    source_contract = _load(implementation_contract)
    if validate_source_contract(source_contract):
        failures.append(implementation_contract.name)
    golden_header = source_fixture_root / "golden" / "call_center_interactions.header.csv"
    if _validate_golden_header(source_contract, golden_header):
        failures.append(golden_header.name)
    snapshot = _load(source_fixture_root / "valid-source-snapshot.json")
    contract_ref = snapshot["sources"][0]["source_contract_ref"]
    if contract_ref["digest"] != _sha256_ref(implementation_contract):
        failures.append("source-contract-digest-mismatch")
    spec22_errors = validate_spec22_coverage(strict=args.strict_spec22)
    report = spec22_coverage_report()
    if spec22_errors:
        failures.extend(spec22_errors)
    elif not args.strict_spec22:
        missing = ", ".join(report["missing_wire_contracts"])
        if missing:
            print(f"Spec-22 inventory: {report['family_count']}/11 families present; wire contracts pending: {missing}")
    if failures:
        print("artifact-envelope fixture validation failed:", ", ".join(failures))
        return 1
    print("artifact-envelope fixtures: valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
