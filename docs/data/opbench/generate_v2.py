"""Generate immutable OPBENCH-lite v2 aggregate artifacts from local sources."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Iterator

from opbench import validate_safe_pack
from opbench_v2 import AGENT_OUTLIER_CONTROL, generate_v2_from_rows, validate_source_coverage_v2
from json_schema import validate as validate_json_schema


TABLE_CONTRACTS = {
    "call_center_interactions": ("interaction_id", {"interaction_id", "customer_id", "reason_category", "channel", "was_resolved", "interaction_date"}),
    "complaints": ("complaint_id", {"complaint_id", "customer_id", "category", "status", "sla_breached"}),
    "satisfaction_surveys": ("survey_id", {"survey_id", "interaction_id", "customer_id", "survey_type", "send_channel", "main_score"}),
    "digital_events": ("event_id", {"event_id", "event_type", "action"}),
    "campaign_sends": ("send_id", {"send_id", "customer_id", "send_channel"}),
    "customers": ("customer_id", {"customer_id", "accepts_marketing"}),
}


def _digest(value: str) -> bytes:
    return hashlib.sha256(value.encode("utf-8")).digest()


def _table_files(root: Path, table: str) -> list[Path]:
    if table not in TABLE_CONTRACTS:
        raise ValueError("unsupported v2 table")
    if table == "customers":
        files = [root / "customers.csv"] if (root / "customers.csv").is_file() else sorted((root / table).rglob("*.csv"))
    else:
        files = sorted((root / table).rglob("*.csv"))
    if not files:
        raise ValueError(f"required source table unavailable: {table}")
    return files


def iter_unique_source_rows(root: Path, table: str) -> Iterator[dict[str, str]]:
    """Yield exact-key-deduplicated rows; only digests are retained for dedupe."""
    primary_key, required = TABLE_CONTRACTS[table]
    connection = sqlite3.connect("")
    try:
        connection.execute("PRAGMA journal_mode=OFF")
        connection.execute("PRAGMA synchronous=OFF")
        connection.execute("CREATE TABLE seen (key_digest BLOB PRIMARY KEY, row_digest BLOB NOT NULL)")
        expected_header: list[str] | None = None
        for path in _table_files(root, table):
            with path.open("r", encoding="utf-8-sig", newline="") as stream:
                reader = csv.DictReader(stream)
                header = reader.fieldnames or []
                if len(header) != len(set(header)) or not required.issubset(header):
                    raise ValueError(f"source schema mismatch in {table}")
                if expected_header is None:
                    expected_header = header
                elif header != expected_header:
                    raise ValueError(f"inconsistent partition schema in {table}")
                for row in reader:
                    key = (row.get(primary_key) or "").strip()
                    if not key:
                        raise ValueError(f"missing primary key in {table}")
                    key_digest = _digest(key)
                    row_digest = _digest(json.dumps(row, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
                    prior = connection.execute(
                        "SELECT row_digest FROM seen WHERE key_digest = ?", (key_digest,)
                    ).fetchone()
                    if prior is not None:
                        if prior[0] != row_digest:
                            raise ValueError(f"conflicting duplicate key in {table}")
                        continue
                    connection.execute(
                        "INSERT INTO seen (key_digest, row_digest) VALUES (?, ?)",
                        (key_digest, row_digest),
                    )
                    yield row
    finally:
        connection.close()


def load_customer_consent(root: Path) -> dict[bytes, str]:
    """Keep only SHA-256 customer-key digests and the consent flag in memory."""
    result: dict[bytes, str] = {}
    for row in iter_unique_source_rows(root, "customers"):
        key = (row.get("customer_id") or "").strip()
        if key:
            result[_digest(key)] = row.get("accepts_marketing", "")
    return result


def write_v2_outputs(output_dir: Path, payload: dict, audit: dict) -> tuple[Path, Path]:
    """Create results/v2 exactly once; existing versioned outputs are immutable."""
    validate_v2_artifacts(payload, audit)
    validate_safe_pack(payload)
    validate_safe_pack(audit)
    target = output_dir / "v2"
    if target.exists():
        raise FileExistsError("v2 output version already exists; refusing to overwrite")
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="opbench-v2-", dir=output_dir) as temporary:
        staged = Path(temporary) / "v2"
        staged.mkdir()
        payload_path = staged / "opbench-lite.json"
        audit_path = staged / "cell_audit.json"
        payload_path.write_text(json.dumps(payload, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")
        audit_path.write_text(json.dumps(audit, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")
        staged.rename(target)
    return target / "opbench-lite.json", target / "cell_audit.json"


def validate_v2_artifacts(payload: dict, audit: dict) -> None:
    """Check file-level schema/cardinality before any output is made durable."""
    schema_path = Path(__file__).with_name("opbench-lite-v2.schema.json")
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    if payload.get("benchmark") != "OPBENCH-lite" or payload.get("version") != "2":
        raise ValueError("v2 payload version or benchmark is invalid")
    if len(payload.get("entries", [])) != schema["properties"]["entries"]["minItems"]:
        raise ValueError("v2 payload does not match the registered entry cardinality")
    if audit.get("benchmark") != "OPBENCH-lite" or audit.get("preregistration") != "discovery_v2.md":
        raise ValueError("v2 audit provenance is invalid")
    _validate_finite_json_numbers(payload)
    _validate_finite_json_numbers(audit)
    _validate_audit_shape(audit)
    if audit.get("planned_cell_count") != 95 or len(audit.get("cells", [])) != 95:
        raise ValueError("v2 audit does not enumerate the registered 95-cell family")
    inferential = [entry for entry in payload["entries"] if entry.get("metric_id") in {"M1", "M2", "M3", "M4", "M5", "M6", "E1"}]
    descriptive = [entry for entry in payload["entries"] if entry.get("metric_id") in {"D1", "R1", "L1"}]
    if len(inferential) != 95 or len(descriptive) != 3:
        raise ValueError("v2 payload must contain 95 inferential cells and three context entries")
    if payload.get("negative_controls", {}).get("agent_outliers", {}).get("aggregate_statement") != AGENT_OUTLIER_CONTROL:
        raise ValueError("v2 audited agent control statement does not match the frozen contract")
    validate_source_coverage_v2(payload.get("source_coverage"))
    validate_json_schema(payload, schema)
    validate_safe_pack(payload)
    validate_safe_pack(audit)


def _validate_finite_json_numbers(value: object) -> None:
    """Reject NaN and infinities, which Python's JSON encoder otherwise emits."""
    if isinstance(value, dict):
        for child in value.values():
            _validate_finite_json_numbers(child)
    elif isinstance(value, (list, tuple)):
        for child in value:
            _validate_finite_json_numbers(child)
    elif isinstance(value, float) and not math.isfinite(value):
        raise ValueError("artifacts may contain only finite JSON numbers")


def _validate_audit_shape(audit: dict) -> None:
    """Fail closed on unregistered audit keys without constraining row values."""
    top_level = {"benchmark", "preregistration", "planned_cell_count", "cells"}
    cell_fields = {
        "_complementary_suppression", "baseline_denominator", "baseline_numerator", "cell",
        "cells_explored", "dbn", "dbx", "denominator", "discovery_p_value", "dn", "dx",
        "effect", "k_min_ok", "metric", "metric_id", "multiple_testing", "numerator",
        "rbn", "rbx", "reason", "replicated", "replication", "rn", "rx", "sn",
        "snapshot", "status", "suppressed", "sx",
    }
    nested_fields = {
        "cell": {"channel", "pqr_category", "reason_category", "scope"},
        "cells_explored": {"family_size"},
        "effect": {"difference", "ci95_high", "ci95_low"},
        "multiple_testing": {"adjusted_q", "family_size", "method"},
        "replication": {"adjusted_q", "baseline_denominator", "baseline_numerator", "denominator", "effect", "numerator", "p_value"},
        "snapshot": {"denominator", "numerator", "rate", "suppressed", "suppression_reason"},
    }
    required_rows = {
        "cell", "cells_explored", "denominator",
        "discovery_p_value", "effect", "k_min_ok", "metric", "metric_id", "multiple_testing",
        "numerator", "reason", "replicated", "replication", "snapshot", "status",
    }
    required_nested = {
        "cells_explored": {"family_size"},
        "multiple_testing": {"adjusted_q", "family_size", "method"},
        "replication": {"adjusted_q", "denominator", "effect", "numerator", "p_value"},
        "snapshot": {"denominator", "numerator", "rate", "suppressed", "suppression_reason"},
    }
    if set(audit) != top_level:
        raise ValueError("audit has unexpected fields")
    if not isinstance(audit.get("cells"), list):
        raise ValueError("audit cells must be a list")
    for row in audit["cells"]:
        if not isinstance(row, dict):
            raise ValueError("audit cell must be an object")
        if set(row) - cell_fields:
            raise ValueError("audit cell has unexpected fields")
        if required_rows - set(row):
            raise ValueError("audit cell is missing required fields")
        # E1 is a descriptive linkage cell and has no discovery/replication baseline.
        if row["metric_id"] != "E1" and {"baseline_denominator", "baseline_numerator"} - set(row):
            raise ValueError("audit inferential cell is missing baseline fields")
        for field, allowed in nested_fields.items():
            child = row.get(field)
            if child is not None and not isinstance(child, dict):
                raise ValueError(f"audit cell {field} must be an object")
            if isinstance(child, dict) and set(child) - allowed:
                raise ValueError(f"audit cell {field} has unexpected fields")
            if field in required_nested and isinstance(child, dict) and required_nested[field] - set(child):
                raise ValueError(f"audit cell {field} is missing required fields")
        if row["metric_id"] != "E1" and isinstance(row.get("replication"), dict) and {
            "baseline_denominator", "baseline_numerator"
        } - set(row["replication"]):
            raise ValueError("audit inferential replication is missing baseline fields")
        if not row["cell"] or any(not isinstance(value, str) for value in row["cell"].values()):
            raise ValueError("audit cell dimensions must contain registered string values")
        if row["effect"] is not None and set(row["effect"]) != nested_fields["effect"]:
            raise ValueError("audit cell effect is missing required fields")


def _run_e0_v2(data_dir: Path, complaint_hashes: set[bytes]) -> dict:
    if not complaint_hashes:
        raise ValueError("bank complaint digest source is empty")
    manifest = Path(__file__).parent / "e0-export" / "Cargo.toml"
    with tempfile.TemporaryDirectory(prefix="opbench-e0-join-") as temporary:
        digest_file = Path(temporary) / "bank-complaint-digests.txt"
        digest_file.write_text("".join(value.hex() + "\n" for value in sorted(complaint_hashes)), encoding="ascii")
        completed = subprocess.run(
            ["cargo", "run", "--offline", "--quiet", "--manifest-path", str(manifest), "--", str(data_dir), "--v2", str(digest_file)],
            check=True, capture_output=True, text=True,
        )
    result = json.loads(completed.stdout)
    if set(result) != {"discovery", "replication", "complaint_ids_matched", "eligible_cases"}:
        raise ValueError("E0 exporter returned unexpected v2 aggregate contract")
    return result


def generate(data_root: Path, e0_data: Path, output_dir: Path) -> tuple[Path, Path]:
    customer_consent = load_customer_consent(data_root)
    complaints = iter_unique_source_rows(data_root, "complaints")
    # The E0 bridge receives only keyed digests, never the source identifiers.
    complaint_hashes = {
        _digest((row.get("complaint_id") or "").strip())
        for row in iter_unique_source_rows(data_root, "complaints")
        if (row.get("complaint_id") or "").strip()
    }
    e0 = _run_e0_v2(e0_data, complaint_hashes)
    payload, audit = generate_v2_from_rows(
        iter_unique_source_rows(data_root, "call_center_interactions"),
        complaints,
        iter_unique_source_rows(data_root, "satisfaction_surveys"),
        iter_unique_source_rows(data_root, "digital_events"),
        iter_unique_source_rows(data_root, "campaign_sends"),
        customer_consent,
        e0,
    )
    return write_v2_outputs(output_dir, payload, audit)


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate immutable aggregate-only OPBENCH-lite v2 outputs")
    parser.add_argument("--data-root", type=Path, required=True)
    parser.add_argument("--e0-data", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, default=Path(__file__).parent / "results")
    arguments = parser.parse_args()
    payload_path, audit_path = generate(arguments.data_root.resolve(), arguments.e0_data.resolve(), arguments.output_dir.resolve())
    print(json.dumps({"payload": payload_path.name, "audit": audit_path.name, "version": "2"}, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        detail = str(exc) if isinstance(exc, (ValueError, FileExistsError)) else type(exc).__name__
        print(f"OPBENCH v2 generation failed safely: {detail}", file=sys.stderr)
        raise SystemExit(2)
