"""Regenerate OPBENCH-lite from local bank CSV partitions and E0 Parquet."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Iterator

from opbench import (
    E0Case,
    K_MIN,
    METRIC_LABELS,
    MetricAccumulator,
    assess_accumulator,
    assess_e1_counts,
    benjamini_hochberg,
    bucket_for_key,
    normalize_channel,
    normalize_pqr_category,
    normalize_reason,
    planned_cells,
    validate_safe_pack,
)


TABLES = ("call_center_interactions", "complaints", "satisfaction_surveys")
EXPECTED_COLUMNS = {
    "call_center_interactions": {"interaction_id", "customer_id", "reason_category", "channel", "was_resolved"},
    "complaints": {"complaint_id", "customer_id", "category", "status", "sla_breached"},
    "satisfaction_surveys": {"survey_id", "interaction_id", "customer_id", "survey_type", "send_channel", "main_score"},
}
BOOLEAN_TRUE = {"true", "1", "yes", "y"}
BOOLEAN_FALSE = {"false", "0", "no", "n"}
OPEN_STATUSES = {"open", "in process", "escalated"}
REASON_ORDER = (
    "complaint", "transactional", "technical", "general_inquiry", "product",
    "account", "card", "loan", "other", "unclassified",
)


def _digest(value: str) -> bytes:
    return hashlib.sha256(value.encode("utf-8")).digest()


def _iter_unique_rows(root: Path, table: str) -> Iterator[dict[str, str]]:
    files = sorted((root / table).rglob("*.csv"))
    if not files:
        raise ValueError(f"required source table unavailable: {table}")
    primary_key = {
        "call_center_interactions": "interaction_id",
        "complaints": "complaint_id",
        "satisfaction_surveys": "survey_id",
    }[table]
    seen: dict[bytes, bytes] = {}
    expected_header: list[str] | None = None
    for path in files:
        with path.open("r", encoding="utf-8-sig", newline="") as stream:
            reader = csv.DictReader(stream)
            header = reader.fieldnames or []
            if len(header) != len(set(header)) or not EXPECTED_COLUMNS[table].issubset(header):
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
                prior = seen.get(key_digest)
                if prior is not None:
                    if prior != row_digest:
                        raise ValueError(f"conflicting duplicate key in {table}")
                    continue
                seen[key_digest] = row_digest
                yield row


def _flag(value: str | None) -> bool | None:
    folded = (value or "").strip().casefold()
    if folded in BOOLEAN_TRUE:
        return True
    if folded in BOOLEAN_FALSE:
        return False
    return None


def _event_and_split(customer_id: str | None) -> str | None:
    customer_id = (customer_id or "").strip()
    if not customer_id:
        return None
    return bucket_for_key("opbench-lite:v1:bank:", customer_id)


def _reason(row: dict[str, str]) -> str:
    # Deliberately do not fall back to free-text contact_reason.
    return normalize_reason(row.get("reason_category"))


def _add(acc: MetricAccumulator, cell: dict[str, str], event: bool, split: str | None) -> None:
    acc.add(cell, event, "overall")
    if split in ("discovery", "replication"):
        acc.add(cell, event, split)


def _channel_cell(channel: str | None) -> str:
    # Private sentinel contributes to totals but cannot match any published cell.
    return channel if channel else "__missing_not_segmented__"


def aggregate_bank(root: Path) -> tuple[dict[str, MetricAccumulator], dict[str, int]]:
    metrics = {metric_id: MetricAccumulator() for metric_id in ("M1", "M2", "M3", "M4", "M5", "M6")}
    interaction_map: dict[bytes, tuple[str, str | None]] = {}
    contacts_seen = Counter()
    for row in _iter_unique_rows(root, "call_center_interactions"):
        contacts_seen["rows"] += 1
        split = _event_and_split(row.get("customer_id"))
        reason = _reason(row)
        channel_raw = (row.get("channel") or "").strip()
        channel = normalize_channel(channel_raw) if channel_raw else None
        interaction_id = (row.get("interaction_id") or "").strip()
        customer_key = (row.get("customer_id") or "").strip()
        interaction_map[_digest(interaction_id)] = (
            reason, split, _digest(customer_key) if customer_key else None,
        )

        resolved = _flag(row.get("was_resolved"))
        if resolved is not None:
            _add(metrics["M1"], {"reason_category": reason, "channel": _channel_cell(channel)}, not resolved, split)
        _add(metrics["M2"], {"channel": _channel_cell(channel)}, reason == "complaint", split)
        if resolved is False:
            _add(metrics["M3"], {"channel": _channel_cell(channel)}, reason == "complaint", split)

    pqr_seen = Counter()
    for row in _iter_unique_rows(root, "complaints"):
        split = _event_and_split(row.get("customer_id"))
        category = normalize_pqr_category(row.get("category"))
        status = (row.get("status") or "").strip().casefold()
        if status in {"open", "in process", "escalated", "resolved", "closed", "rejected"}:
            pqr_seen["status_valid"] += 1
            _add(metrics["M4"], {"pqr_category": category}, status in OPEN_STATUSES, split)
        breached = _flag(row.get("sla_breached"))
        if breached is not None:
            pqr_seen["sla_flag_valid"] += 1
            _add(metrics["M5"], {"pqr_category": category}, breached, split)

    survey_seen = Counter()
    for row in _iter_unique_rows(root, "satisfaction_surveys"):
        if (row.get("survey_type") or "").strip().casefold() != "csat":
            continue
        try:
            score = int((row.get("main_score") or "").strip())
        except ValueError:
            continue
        if not 1 <= score <= 5:
            continue
        survey_seen["eligible_csat"] += 1
        linked = interaction_map.get(_digest((row.get("interaction_id") or "").strip()))
        if linked is None:
            continue
        reason, contact_split, contact_customer_digest = linked
        survey_customer_id = (row.get("customer_id") or "").strip()
        survey_customer_digest = _digest(survey_customer_id) if survey_customer_id else None
        if contact_customer_digest and survey_customer_digest and contact_customer_digest != survey_customer_digest:
            continue
        survey_seen["linked_eligible_csat"] += 1
        split = contact_split or _event_and_split(survey_customer_id)
        channel_raw = (row.get("send_channel") or "").strip()
        channel = normalize_channel(channel_raw) if channel_raw else None
        cell = {"reason_category": reason, "channel": _channel_cell(channel)}
        _add(metrics["M6"], cell, score <= 2, split)

    return metrics, {
        "contact_rows_deduplicated": contacts_seen["rows"],
        "pqr_rows_with_valid_status": pqr_seen["status_valid"],
        "pqr_rows_with_valid_sla_flag": pqr_seen["sla_flag_valid"],
        "eligible_csat": survey_seen["eligible_csat"],
        "linked_eligible_csat": survey_seen["linked_eligible_csat"],
    }


def _run_e0(data_dir: Path) -> dict[str, dict[str, int]]:
    manifest = Path(__file__).parent / "e0-export" / "Cargo.toml"
    completed = subprocess.run(
        ["cargo", "run", "--offline", "--quiet", "--manifest-path", str(manifest), "--", str(data_dir)],
        check=True, capture_output=True, text=True,
    )
    value = json.loads(completed.stdout)
    for key in ("discovery", "replication"):
        if set(value[key]) != {"selected", "total"}:
            raise ValueError("E0 exporter returned unexpected aggregate contract")
    return value


def _metric_rows(metrics: dict[str, MetricAccumulator], e0_data: Path) -> list[dict]:
    rows: list[dict] = []
    for metric_id, accumulator in metrics.items():
        cells = planned_cells(metric_id)
        rows.extend(assess_accumulator(metric_id, cells, accumulator, family_size=181))
    e0 = _run_e0(e0_data.resolve())
    rows.append(assess_e1_counts(
        e0["discovery"]["selected"], e0["discovery"]["total"],
        e0["replication"]["selected"], e0["replication"]["total"],
        family_size=181,
    ))
    return rows


def _apply_global_tests(rows: list[dict]) -> None:
    """Apply the one preregistered 181-cell discovery and frozen-candidate families."""
    discovery_q = benjamini_hochberg([row.get("discovery_p_value") for row in rows])
    candidates = []
    for row, q_value in zip(rows, discovery_q):
        row["multiple_testing"]["family_size"] = len(rows)
        row["multiple_testing"]["adjusted_q"] = q_value if row.get("discovery_p_value") is not None else None
        if row["cell"] == {"scope": "overall"}:
            continue
        effect = row.get("effect")
        delta = effect["difference"] if effect else None
        if row["k_min_ok"] and delta is not None and abs(delta) >= 0.05:
            candidates.append(row)

    replication_q = benjamini_hochberg([
        row.get("replication", {}).get("p_value") for row in candidates
    ]) if candidates else []
    replication_q_by_id = {id(row): q for row, q in zip(candidates, replication_q)}
    for row in rows:
        if row["cell"] == {"scope": "overall"}:
            continue
        if not row["k_min_ok"]:
            row.update(status="uncertain", reason="k_support_not_met", replicated=False)
            continue
        effect = row.get("effect")
        delta = effect["difference"] if effect else None
        rep_effect = row.get("replication", {}).get("effect")
        rep_delta = rep_effect["difference"] if rep_effect else None
        q_value = row["multiple_testing"]["adjusted_q"]
        discovery_candidate = delta is not None and abs(delta) >= 0.05 and q_value is not None and q_value <= 0.05
        discovery_descriptive = delta is not None and abs(delta) >= 0.05 and not discovery_candidate
        if not discovery_candidate and not discovery_descriptive:
            if delta is not None and rep_delta is not None and abs(delta) < 0.05 and abs(rep_delta) < 0.05:
                row.update(status="refuted", reason="minimum_effect_not_observed", replicated=False)
            else:
                row.update(status="uncertain", reason="discovery_replication_disagree", replicated=False)
            continue
        rep_q = replication_q_by_id.get(id(row))
        row["replication"]["adjusted_q"] = rep_q
        if rep_delta is None or delta is None or rep_delta * delta <= 0 or abs(rep_delta) < 0.05:
            row.update(status="refuted", reason="effect_or_direction_did_not_replicate", replicated=False)
        elif discovery_candidate and rep_q is not None and rep_q <= 0.05:
            row.update(status="corroborated", reason="independent_replication_passed", replicated=True)
        else:
            row.update(status="corroborated_descriptive", reason="effect_replicated_without_adjusted_significance", replicated=True)

    # E1 is itself the split comparison; its adjusted q is from the full 181-cell
    # discovery family, and it has no second-stage candidate test.
    e1 = next((row for row in rows if row["metric_id"] == "E1"), None)
    if e1 is not None:
        e1_q = e1["multiple_testing"]["adjusted_q"]
        delta = (e1.get("effect") or {}).get("difference")
        if not e1["k_min_ok"]:
            e1.update(status="uncertain", reason="k_support_not_met", replicated=False)
        elif delta is not None and delta >= 0.05 and e1_q is not None and e1_q <= 0.05:
            e1.update(status="corroborated", reason="modal_signature_increased_in_replication", replicated=True)
        elif delta is not None and delta <= -0.05 and e1_q is not None and e1_q <= 0.05:
            e1.update(status="refuted", reason="modal_signature_decreased_in_replication", replicated=False)
        elif delta is not None and abs(delta) < 0.05:
            e1.update(status="corroborated_descriptive", reason="modal_signature_rate_stable_across_splits", replicated=True)
        else:
            e1.update(status="uncertain", reason="replication_difference_not_significant", replicated=False)
        e1["replication"]["adjusted_q"] = e1_q


def _catalog_text(row: dict) -> tuple[str, list[str], list[str]]:
    cell = row["cell"]
    metric_id = row["metric_id"]
    if metric_id == "M1":
        title = f"First-call unresolved rate is higher for {cell['channel']} contacts tagged {cell['reason_category']}"
        caveats = ["The comparison is against the pooled complement of this metric, not a causal control.", "The source flag measures first-call resolution only; the complaint tag is not a root cause."]
        tags = ["flow", "decision_model", "agent"]
    elif metric_id == "M6":
        title = f"Low CSAT is more common for linked {cell['reason_category']} contacts surveyed via {cell['channel']}"
        caveats = ["Low CSAT is score 1–2 on a 1–5 scale; this is not NPS.", "Survey channel 'other' includes unsupported delivery labels; association does not establish cause."]
        tags = ["flow", "decision_model", "agent"]
    elif metric_id == "E1":
        title = "The discovery-leading E0 query signature appears at a similar rate in replication"
        caveats = ["This is a repeated opaque query signature, not repeated customer intent or a causal mechanism.", "The discovery-to-replication difference is below the preregistered five-percentage-point threshold."]
        tags = []
    elif metric_id == "M2":
        title = f"Complaint-tagged contacts are not materially overrepresented in {cell['channel']}"
        caveats = ["The cell did not meet the preregistered minimum-effect rule; this does not mean complaints are absent."]
        tags = []
    elif metric_id == "M3":
        title = f"Complaint tags do not form a materially higher share of unresolved {cell['channel']} contacts"
        caveats = ["This is a first-call unresolved subset and a coarse category association, not an explanation of why contacts failed."]
        tags = []
    elif metric_id == "M4":
        title = f"Open-state PQR share does not materially differ for {cell['pqr_category']} PQRs"
        caveats = ["Open-state is a final-extract status snapshot (open, in process, or escalated), not status history."]
        tags = []
    else:
        title = f"Observed SLA-breach flag rate does not materially differ for {cell['pqr_category']} PQRs"
        caveats = ["The data dictionary has no SLA deadline or eligibility field; this is the supplied flag, not an independent SLA audit."]
        tags = []
    return title, caveats, tags


def _coverage_count(value: int, name: str) -> dict:
    return {
        "value": value if value >= K_MIN else None,
        "suppressed": value < K_MIN,
        "suppression_reason": None if value >= K_MIN else f"{name}_below_k",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Regenerate deterministic aggregate OPBENCH-lite artifacts")
    parser.add_argument("--data-root", type=Path, required=True, help="local root containing the three named bank tables")
    parser.add_argument("--e0-data", type=Path, required=True, help="local E0 datos directory")
    parser.add_argument("--output-dir", type=Path, default=Path(__file__).parent / "results")
    args = parser.parse_args()

    metrics, source_coverage = aggregate_bank(args.data_root.resolve())
    rows = _metric_rows(metrics, args.e0_data)
    rows.sort(key=lambda row: (row["metric_id"], json.dumps(row["cell"], sort_keys=True)))
    _apply_global_tests(rows)
    positives = [row for row in rows if row["cell"] != {"scope": "overall"} and row["status"] in {"candidate", "candidate_descriptive", "corroborated", "corroborated_descriptive"}]
    selected: list[dict] = []
    for metric_id, limit in (("M1", 2), ("M6", 1)):
        metric_rows = [
            row for row in positives
            if row["metric_id"] == metric_id
            and (row.get("effect") or {}).get("difference", 0) > 0
            and row["cell"].get("channel") != "other"
        ]
        selected.extend(sorted(metric_rows, key=lambda row: (-row["effect"]["difference"], json.dumps(row["cell"], sort_keys=True)))[:limit])
    e1_row = next((row for row in rows if row["metric_id"] == "E1" and row["status"] == "corroborated_descriptive"), None)
    if e1_row is not None:
        selected.append(e1_row)
    supported_nonfindings = [row for row in rows if row["cell"] != {"scope": "overall"} and row["status"] == "refuted" and row["k_min_ok"]]
    for metric_id in ("M2", "M3", "M4", "M5"):
        candidates = [row for row in supported_nonfindings if row["metric_id"] == metric_id]
        if candidates:
            selected.append(min(candidates, key=lambda row: (abs((row.get("effect") or {}).get("difference", 0)), json.dumps(row["cell"], sort_keys=True))))
    selected = selected[:8]
    if len(selected) < 6 or len([row for row in selected if row["status"] in {"refuted", "uncertain"}]) < 3:
        raise ValueError("fixed pre-registered registry does not yield 6-8 sufficiently supported benchmark entries")

    entries = []
    for index, row in enumerate(selected, start=1):
        status = row["status"]
        title, caveats, tags = _catalog_text(row)
        proposal_notes = (
            [
                "Use this as a measured signal to focus a bounded proposal on the named normalized population and preserve the outcome definition.",
                "Do not claim that the reason or channel caused the outcome; any benefit estimate requires a separate validated scenario.",
            ] if status in {"candidate", "candidate_descriptive", "corroborated"}
            else [
                "Do not prioritize a targeted change for this cell from the current evidence; the registered material-difference rule was not met.",
                "Keep this result as a negative benchmark so future proposals do not overfit noise or turn the category into a cause.",
            ]
        )
        entries.append({
            "id": f"OPB-{index:02d}",
            "family": "attention_contact" if row["metric_id"] in {"M1", "M2", "M3", "M6"} else "pqr_sla" if row["metric_id"] in {"M4", "M5"} else "e0_operations",
            "title": title,
            "population": "pre-registered aggregate cell in the synthetic hackathon snapshot",
            "metric_id": row["metric_id"], "cell": row["cell"],
            "numerator": row["snapshot"]["numerator"], "denominator": row["snapshot"]["denominator"],
            "snapshot": row["snapshot"],
            "discovery": {
                "numerator": row["numerator"], "denominator": row["denominator"],
                "baseline_numerator": row.get("baseline_numerator"),
                "baseline_denominator": row.get("baseline_denominator"),
                "effect": row["effect"],
                "adjusted_q": row["multiple_testing"]["adjusted_q"],
            },
            "effect": row["effect"],
            "effect_basis": "discovery-selected modal signature rate: replication minus discovery" if row["metric_id"] == "E1" else "discovery cell versus its pooled complement; replicated independently in the second hash split",
            "replicated": row["replicated"],
            "replication": row["replication"], "cells_explored": row["cells_explored"],
            "multiple_testing": row["multiple_testing"], "k_min_ok": row["k_min_ok"],
            "status": status,
            "type": "problem" if status in {"candidate", "corroborated"} else "descriptive_only" if status == "corroborated_descriptive" else "descriptive_only",
            "mechanism_class": "observed_outcome_association_hypothesis_only",
            "improvement_tags": tags,
            "adequate_proposal_notes": proposal_notes,
            "caveats": caveats + ["Synthetic hackathon snapshot; not real-bank prevalence or causal evidence."],
        })

    eligible = source_coverage["eligible_csat"]
    linked = source_coverage["linked_eligible_csat"]
    linkage_support = linked >= K_MIN and eligible - linked >= K_MIN
    coverage = {
        "eligible_csat": eligible if linkage_support else None,
        "linked_eligible_csat": linked if linkage_support else None,
        "link_rate": round(linked / eligible, 6) if linkage_support else None,
        "suppressed": not linkage_support,
        "suppression_reason": None if linkage_support else "unlinked_or_identity_mismatch_below_k",
    }
    source_coverage = {
        "contact_rows_deduplicated": _coverage_count(source_coverage["contact_rows_deduplicated"], "contact_rows_deduplicated"),
        "pqr_rows_with_valid_status": _coverage_count(source_coverage["pqr_rows_with_valid_status"], "pqr_rows_with_valid_status"),
        "pqr_rows_with_valid_sla_flag": _coverage_count(source_coverage["pqr_rows_with_valid_sla_flag"], "pqr_rows_with_valid_sla_flag"),
        "csat_linkage": coverage,
    }
    payload = {
        "benchmark": "OPBENCH-lite", "version": "1.0.0",
        "interpretation": "Synthetic hackathon dataset; descriptive association and split replication only, not causal or production impact evidence.",
        "inputs": {"bank_tables": list(TABLES), "e0_tables": ["case", "copilot_query"]},
        "privacy": {"minimum_count": K_MIN, "aggregate_only": True, "row_data_included": False},
    "source_coverage": source_coverage,
        "entries": entries,
    }
    audit = {
        "benchmark": "OPBENCH-lite", "preregistration": "discovery_v1.md",
        "planned_cell_count": 181,
        "cells": rows,
    }
    validate_safe_pack(payload)
    validate_safe_pack(audit)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for name, value in (("opbench-lite.json", payload), ("cell_audit.json", audit)):
        output = args.output_dir / name
        output.write_text(json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"entries": len(entries), "cells_explored": len(rows), "positive": len(positives), "supported_nonfindings": len(supported_nonfindings)}, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        # ValueError messages here are fixed, authored validation reasons; no row is interpolated.
        detail = str(exc) if isinstance(exc, ValueError) else type(exc).__name__
        print(f"OPBENCH generation failed safely: {detail}", file=sys.stderr)
        raise SystemExit(2)
