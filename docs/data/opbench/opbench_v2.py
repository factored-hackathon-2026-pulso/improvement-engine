"""Frozen v2 vocabularies and aggregate-only descriptive measures.

This module is intentionally separate from the v1 statistics implementation:
v1 output remains a historical, reproducible artifact while v2 adopts the
independently audited source domains.
"""

from __future__ import annotations

import unicodedata
from collections.abc import Iterable, Mapping
from datetime import datetime
from typing import Any

from opbench import (
    MetricAccumulator,
    assess_accumulator,
    assess_e1_counts,
    benjamini_hochberg,
    bucket_for_key,
    difference_interval,
    two_proportion_pvalue,
    validate_safe_pack,
)


K_MIN = 10
REASONS_V2 = ("complaint", "transactional", "technical", "commercial", "retention", "product")
CHANNELS_V2 = ("phone", "email", "mobile_app", "whatsapp", "web_chat", "web")
PQR_CATEGORIES_V2 = ("transactions", "fees", "technical", "branch", "service")
SURVEY_CHANNELS_V2 = ("email", "phone", "mobile_app", "web", "other")
METRICS_V2 = ("M1", "M2", "M3", "M4", "M5", "M6", "E1")
FAMILY_SIZE_V2 = 95
E0_DISCOVERY_CASES_V2 = 200
E0_REPLICATION_CASES_V2 = 1800
E0_ELIGIBLE_CASES_V2 = E0_DISCOVERY_CASES_V2 + E0_REPLICATION_CASES_V2
AGENT_OUTLIER_CONTROL = (
    "Audit 2026-10-04: 1,090 agents; resolution/escalation/follow-up dispersion "
    "ratios 0.97/0.99/0.96; no outlier evidence. Audit-derived, not recomputed by OPBENCH-lite v2."
)

_REASON_ALIASES = {
    "queja": "complaint", "reclamo": "complaint", "complaint": "complaint",
    "transaccional": "transactional", "transactional": "transactional", "transaction": "transactional",
    "tecnico": "technical", "technical": "technical",
    "comercial": "commercial", "commercial": "commercial",
    "retencion": "retention", "retention": "retention",
    "producto": "product", "product": "product",
}
_CHANNEL_ALIASES = {
    "phone": "phone", "email": "email", "app": "mobile_app",
    "whatsapp": "whatsapp", "web_chat": "web_chat", "web chat": "web_chat", "web": "web",
}
_PQR_ALIASES = {
    "transactions": "transactions", "fees": "fees", "technical": "technical",
    "branch": "branch", "service": "service",
}
_SURVEY_CHANNELS = {
    "email": "email", "ivr": "phone", "app": "mobile_app", "web": "web",
}
_DIGITAL_ACTION_GROUPS = {
    "transactional_actions": frozenset(("view_transactions", "initiate_transfer", "initiate_payment")),
    "other_views": frozenset(("view_help", "view_home", "view_accounts", "view_products")),
}
_MARKETING_CHANNELS = frozenset(("email", "push", "sms", "voice", "whatsapp"))
_BOOLEAN_TRUE = frozenset(("true", "1", "yes", "y"))
_BOOLEAN_FALSE = frozenset(("false", "0", "no", "n"))
_OPEN_STATUSES = frozenset(("open", "in process", "escalated"))
_VALID_PQR_STATUSES = _OPEN_STATUSES | frozenset(("resolved", "closed", "rejected"))


def _fold(value: str | None) -> str:
    decomposed = unicodedata.normalize("NFKD", (value or "").strip().casefold())
    return "".join(char for char in decomposed if not unicodedata.combining(char))


def normalize_reason_v2(value: str | None) -> str:
    folded = _fold(value)
    if not folded:
        raise ValueError("contact reason is missing")
    try:
        return _REASON_ALIASES[folded]
    except KeyError as exc:
        raise ValueError("unrecognized contact reason") from exc


def normalize_channel_v2(value: str | None) -> str:
    folded = (value or "").strip().casefold()
    if not folded:
        raise ValueError("contact channel is missing")
    try:
        return _CHANNEL_ALIASES[folded]
    except KeyError as exc:
        raise ValueError("unrecognized contact channel") from exc


def normalize_pqr_category_v2(value: str | None) -> str:
    folded = _fold(value)
    if not folded:
        raise ValueError("PQR category is missing")
    try:
        return _PQR_ALIASES[folded]
    except KeyError as exc:
        raise ValueError("unrecognized PQR category") from exc


def normalize_survey_channel_v2(value: str | None) -> str:
    return _SURVEY_CHANNELS.get(_fold(value), "other")


def planned_cells_v2(metric_id: str) -> list[dict[str, str]]:
    """Return the frozen ordered v2 family cells for one inferential metric."""
    if metric_id == "M1":
        cells = [
            {"reason_category": reason, "channel": channel}
            for reason in REASONS_V2 for channel in CHANNELS_V2
        ]
    elif metric_id in ("M2", "M3"):
        cells = [{"channel": channel} for channel in CHANNELS_V2]
    elif metric_id in ("M4", "M5"):
        cells = [{"pqr_category": category} for category in PQR_CATEGORIES_V2]
    elif metric_id == "M6":
        cells = [
            {"reason_category": reason, "channel": channel}
            for reason in REASONS_V2 for channel in SURVEY_CHANNELS_V2
        ]
    elif metric_id == "E1":
        cells = []
    else:
        raise ValueError("unsupported v2 metric")
    return cells + [{"scope": "overall"}]


def _digest(value: str) -> bytes:
    import hashlib

    return hashlib.sha256(value.encode("utf-8")).digest()


def _split(customer_id: str | None) -> str | None:
    key = (customer_id or "").strip()
    return bucket_for_key("opbench-lite:v1:bank:", key) if key else None


def _flag(value: Any) -> bool | None:
    if isinstance(value, bool):
        return value
    if isinstance(value, (int, float)) and value in (0, 1):
        return bool(value)
    if value is None:
        return None
    folded = str(value).strip().casefold()
    if folded in _BOOLEAN_TRUE:
        return True
    if folded in _BOOLEAN_FALSE:
        return False
    return None


def _add_metric(accumulator: MetricAccumulator, cell: Mapping[str, str], event: bool, split: str | None) -> None:
    accumulator.add(cell, event, "overall")
    if split in ("discovery", "replication"):
        accumulator.add(cell, event, split)


def aggregate_bank_v2_rows(
    contacts: Iterable[Mapping[str, Any]],
    pqrs: Iterable[Mapping[str, Any]],
    surveys: Iterable[Mapping[str, Any]],
) -> tuple[dict[str, MetricAccumulator], dict[str, int], dict[tuple[tuple[str, str], ...], bool]]:
    """Aggregate approved bank rows to sufficient statistics only.

    CSV ingestion/deduplication is handled by the v2 source reader; this
    function retains only counters and transient SHA-256 keyed contact linkage.
    """
    metrics = {name: MetricAccumulator() for name in ("M1", "M2", "M3", "M4", "M5", "M6")}
    coverage = {
        "contact_rows": 0, "contacts_with_reason": 0, "contacts_with_channel": 0,
        "pqr_valid_status": 0, "pqr_valid_sla_flag": 0,
        "eligible_csat": 0, "linked_eligible_csat": 0,
    }
    interaction_map: dict[bytes, tuple[str, str | None, bytes | None]] = {}
    monthly_cells: dict[tuple[str, str], dict[str, list[int]]] = {
        (reason, channel): {} for reason in REASONS_V2 for channel in CHANNELS_V2
    }
    full_months = _full_months_v2()
    for row in contacts:
        coverage["contact_rows"] += 1
        reason_raw = str(row.get("reason_category") or "").strip()
        channel_raw = str(row.get("channel") or "").strip()
        reason = None
        channel = None
        if reason_raw:
            reason = normalize_reason_v2(reason_raw)
            coverage["contacts_with_reason"] += 1
        if channel_raw:
            channel = normalize_channel_v2(channel_raw)
            coverage["contacts_with_channel"] += 1
        interaction_id = str(row.get("interaction_id") or "").strip()
        customer_id = str(row.get("customer_id") or "").strip()
        if interaction_id and reason:
            interaction_map[_digest(interaction_id)] = (
                reason, _split(customer_id), _digest(customer_id) if customer_id else None,
            )
        split = _split(customer_id)
        resolved = _flag(row.get("was_resolved"))
        channel_cell = channel if channel is not None else "__missing_not_segmented__"
        if reason is not None:
            if resolved is not None:
                _add_metric(metrics["M1"], {"reason_category": reason, "channel": channel_cell}, not resolved, split)
            _add_metric(metrics["M2"], {"channel": channel_cell}, reason == "complaint", split)
            if resolved is False:
                _add_metric(metrics["M3"], {"channel": channel_cell}, reason == "complaint", split)
            if channel is not None and resolved is not None:
                raw_date = str(row.get("interaction_date") or "").strip()
                month = _month_from_timestamp(raw_date) if raw_date else ""
                if month in full_months:
                    pair = monthly_cells[(reason, channel)].setdefault(month, [0, 0])
                    pair[1] += 1
                    pair[0] += int(not resolved)

    for row in pqrs:
        raw_category = str(row.get("category") or "").strip()
        if not raw_category:
            continue
        category = normalize_pqr_category_v2(raw_category)
        split = _split(str(row.get("customer_id") or ""))
        status = str(row.get("status") or "").strip().casefold()
        if status in _VALID_PQR_STATUSES:
            coverage["pqr_valid_status"] += 1
            _add_metric(metrics["M4"], {"pqr_category": category}, status in _OPEN_STATUSES, split)
        breached = _flag(row.get("sla_breached"))
        if breached is not None:
            coverage["pqr_valid_sla_flag"] += 1
            _add_metric(metrics["M5"], {"pqr_category": category}, breached, split)

    for row in surveys:
        if str(row.get("survey_type") or "").strip().casefold() != "csat":
            continue
        try:
            score = int(str(row.get("main_score") or "").strip())
        except ValueError:
            continue
        if not 1 <= score <= 5:
            continue
        coverage["eligible_csat"] += 1
        interaction_id = str(row.get("interaction_id") or "").strip()
        linked = interaction_map.get(_digest(interaction_id)) if interaction_id else None
        if linked is None:
            continue
        reason, contact_split, contact_customer_digest = linked
        survey_customer_id = str(row.get("customer_id") or "").strip()
        survey_customer_digest = _digest(survey_customer_id) if survey_customer_id else None
        if contact_customer_digest and survey_customer_digest and contact_customer_digest != survey_customer_digest:
            continue
        coverage["linked_eligible_csat"] += 1
        split = contact_split or _split(survey_customer_id)
        channel = normalize_survey_channel_v2(str(row.get("send_channel") or ""))
        _add_metric(metrics["M6"], {"reason_category": reason, "channel": channel}, score <= 2, split)

    monthly_persistence = {}
    for reason in REASONS_V2:
        for channel in CHANNELS_V2:
            values = []
            for month in full_months:
                focal_events, focal_total = monthly_cells[(reason, channel)].get(month, [0, 0])
                complement_events = complement_total = 0
                for other_reason in REASONS_V2:
                    if other_reason == reason:
                        continue
                    events, total = monthly_cells[(other_reason, channel)].get(month, [0, 0])
                    complement_events += events
                    complement_total += total
                values.append((focal_total, focal_events, complement_total, complement_events))
            cell_key = tuple(sorted({"reason_category": reason, "channel": channel}.items()))
            monthly_persistence[cell_key] = m1_monthly_persistent(values)
    return metrics, coverage, monthly_persistence


def _full_months_v2() -> list[str]:
    """The frozen 35 complete calendar months, July 2023 through May 2026."""
    result = []
    year, month = 2023, 7
    for _ in range(35):
        result.append(f"{year:04d}-{month:02d}")
        month += 1
        if month == 13:
            year += 1
            month = 1
    return result


def _month_from_timestamp(value: str) -> str:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        raise ValueError("unparseable contact timestamp") from exc
    return f"{parsed.year:04d}-{parsed.month:02d}"


def assess_m1_v2(accumulator: MetricAccumulator) -> list[dict[str, Any]]:
    """Assess M1 against other reasons in the same channel, not all channels."""
    output = []
    cells = planned_cells_v2("M1")
    for cell in cells:
        overall = cell == {"scope": "overall"}
        focal_key = tuple(sorted(cell.items()))
        focal = accumulator.grouped.get(focal_key, {})

        def pair(split: str, complement: bool = False) -> tuple[int, int]:
            if overall:
                value = accumulator.totals[split]
                return value[0], value[1]
            if not complement:
                value = focal.get(split, [0, 0])
                return value[0], value[1]
            channel = cell["channel"]
            total_events = total_rows = 0
            for key, by_split in accumulator.grouped.items():
                values = dict(key)
                if values.get("channel") == channel and values.get("reason_category") != cell["reason_category"]:
                    pair_value = by_split.get(split, [0, 0])
                    total_events += pair_value[0]
                    total_rows += pair_value[1]
            return total_events, total_rows

        d_num, d_den = pair("discovery")
        db_num, db_den = pair("discovery", complement=True) if not overall else (0, 0)
        r_num, r_den = pair("replication")
        rb_num, rb_den = pair("replication", complement=True) if not overall else (0, 0)
        s_num, s_den = pair("overall")
        sb_num, sb_den = pair("overall", complement=True) if not overall else (0, 0)

        def supported(success: int, total: int) -> bool:
            return success >= K_MIN and total - success >= K_MIN

        d_ok = supported(d_num, d_den) and supported(db_num, db_den) if not overall else supported(d_num, d_den)
        r_ok = supported(r_num, r_den) and supported(rb_num, rb_den) if not overall else supported(r_num, r_den)
        snapshot_ok = supported(s_num, s_den) and (overall or supported(sb_num, sb_den))
        d_effect = d_num / d_den - db_num / db_den if d_ok and not overall else None
        r_effect = r_num / r_den - rb_num / rb_den if r_ok and not overall else None
        d_p = two_proportion_pvalue(d_num, d_den, db_num, db_den) if d_ok and not overall else None
        r_p = two_proportion_pvalue(r_num, r_den, rb_num, rb_den) if r_ok and not overall else None
        d_ci = difference_interval(d_num, d_den, db_num, db_den) if d_ok and not overall else None
        r_ci = difference_interval(r_num, r_den, rb_num, rb_den) if r_ok and not overall else None
        snapshot = _safe_binary_summary(s_num, s_den) if snapshot_ok else {
            "numerator": None, "denominator": None, "rate": None,
            "suppressed": True, "suppression_reason": "event_or_nonevent_below_k",
        }
        output.append({
            "metric_id": "M1", "metric": "contact_unresolved_rate", "cell": cell,
            "status": "uncertain", "reason": "overall_reference_not_tested" if overall else "awaiting_global_family_test",
            "numerator": d_num if d_ok else None, "denominator": d_den if d_ok else None,
            "baseline_numerator": db_num if d_ok and not overall else None,
            "baseline_denominator": db_den if d_ok and not overall else None,
            "snapshot": snapshot,
            "effect": None if d_ci is None else {"difference": round(d_effect, 6), "ci95_low": d_ci[0], "ci95_high": d_ci[1]},
            "discovery_p_value": d_p,
            "multiple_testing": {"method": "two_proportion_z_benjamini_hochberg_fdr", "family_size": FAMILY_SIZE_V2, "adjusted_q": None},
            "replicated": False,
            "replication": {
                "numerator": r_num if r_ok else None, "denominator": r_den if r_ok else None,
                "baseline_numerator": rb_num if r_ok and not overall else None,
                "baseline_denominator": rb_den if r_ok and not overall else None,
                "effect": None if r_ci is None else {"difference": round(r_effect, 6), "ci95_low": r_ci[0], "ci95_high": r_ci[1]},
                "p_value": r_p, "adjusted_q": None,
            },
            "k_min_ok": d_ok and r_ok,
            "cells_explored": {"family_size": FAMILY_SIZE_V2},
            "_snapshot_focal_total": s_den,
            "_snapshot_complement_total": sb_den,
        })
    return output


def _cell_key(row: Mapping[str, Any]) -> tuple[str, tuple[tuple[str, str], ...]]:
    return row["metric_id"], tuple(sorted(row["cell"].items()))


def apply_global_tests_v2(
    rows: list[dict[str, Any]],
    monthly_persistence: Mapping[tuple[tuple[str, str], ...], bool] | None = None,
) -> None:
    """Apply the fixed 95-test family and independent replication gates."""
    if len(rows) != FAMILY_SIZE_V2:
        raise ValueError("v2 global test family must contain exactly 95 cells")
    persistence = monthly_persistence or {}
    discovery_q = benjamini_hochberg([row.get("discovery_p_value") for row in rows])
    candidates: list[dict[str, Any]] = []
    for row, q_value in zip(rows, discovery_q):
        row["multiple_testing"] = {
            "method": "two_proportion_z_benjamini_hochberg_fdr",
            "family_size": FAMILY_SIZE_V2,
            "adjusted_q": q_value if row.get("discovery_p_value") is not None else None,
        }
        row["cells_explored"] = {"family_size": FAMILY_SIZE_V2}
        if row["cell"] == {"scope": "overall"} or not row.get("k_min_ok"):
            continue
        effect = row.get("effect")
        delta = effect.get("difference") if effect else None
        q = row["multiple_testing"]["adjusted_q"]
        if row["metric_id"] == "M1":
            persistent = persistence.get(tuple(sorted(row["cell"].items())), False)
            qualifies = m1_discovery_qualified(
                row.get("numerator") or 0, row.get("denominator") or 0,
                row.get("baseline_numerator") or 0, row.get("baseline_denominator") or 0,
                q, row.get("_snapshot_focal_total", 0), row.get("_snapshot_complement_total", 0),
            ) and persistent
        else:
            qualifies = delta is not None and abs(delta) >= 0.05 and q is not None and q <= 0.05
        row["_discovery_candidate"] = bool(qualifies)
        if qualifies:
            candidates.append(row)

    replication_q = benjamini_hochberg([
        row.get("replication", {}).get("p_value") for row in candidates
    ]) if candidates else []
    rep_q_by_key = {
        _cell_key(row): q for row, q in zip(candidates, replication_q)
    }
    for row in rows:
        if row["cell"] == {"scope": "overall"}:
            row.update(status="uncertain", reason="overall_reference_not_tested", replicated=False)
            continue
        if not row.get("k_min_ok"):
            row.update(status="uncertain", reason="k_support_not_met", replicated=False)
            continue
        if row["metric_id"] == "E1":
            delta = (row.get("effect") or {}).get("difference")
            q = row["multiple_testing"].get("adjusted_q")
            if delta is not None and abs(delta) < 0.05:
                row.update(status="corroborated_descriptive", reason="modal_signature_rate_stable_across_splits", replicated=True)
            elif delta is not None and delta >= 0.05 and q is not None and q <= 0.05:
                row.update(status="corroborated", reason="modal_signature_increased_in_holdout", replicated=True)
            elif delta is not None and delta <= -0.05 and q is not None and q <= 0.05:
                row.update(status="refuted", reason="modal_signature_decreased_in_holdout", replicated=False)
            else:
                row.update(status="uncertain", reason="holdout_difference_not_significant", replicated=False)
            row["replication"]["adjusted_q"] = q
            continue

        discovery_effect = row.get("effect")
        replication_effect = row.get("replication", {}).get("effect")
        d_delta = discovery_effect.get("difference") if discovery_effect else None
        r_delta = replication_effect.get("difference") if replication_effect else None
        d_candidate = row.get("_discovery_candidate", False)
        r_q = rep_q_by_key.get(_cell_key(row))
        row["replication"]["adjusted_q"] = r_q
        if d_candidate:
            if row["metric_id"] == "M1":
                r_num = row["replication"].get("numerator") or 0
                r_den = row["replication"].get("denominator") or 0
                rb_num = row["replication"].get("baseline_numerator") or 0
                rb_den = row["replication"].get("baseline_denominator") or 0
                persistent = persistence.get(tuple(sorted(row["cell"].items())), False)
                replication_effect_ok = m1_replication_effect_qualified(r_num, r_den, rb_num, rb_den)
                replicated = persistent and replication_effect_ok
            else:
                replicated = (
                    d_delta is not None and r_delta is not None and d_delta * r_delta > 0
                    and abs(r_delta) >= 0.05
                )
            if row["metric_id"] == "M1" and replicated and (r_q is None or r_q > 0.05):
                row.update(status="uncertain", reason="replication_fdr_not_met", replicated=False)
            elif row["metric_id"] == "M1" and not persistent:
                row.update(status="uncertain", reason="monthly_persistence_not_confirmed", replicated=False)
            elif not replicated:
                row.update(status="refuted", reason="effect_or_direction_did_not_replicate", replicated=False)
            elif row["metric_id"] == "M1" or (r_q is not None and r_q <= 0.05):
                row.update(status="corroborated", reason="independent_replication_passed", replicated=True)
            else:
                row.update(status="corroborated_descriptive", reason="effect_replicated_without_adjusted_significance", replicated=True)
        elif d_delta is not None and r_delta is not None and abs(d_delta) < 0.05 and abs(r_delta) < 0.05:
            row.update(status="refuted", reason="minimum_effect_not_observed", replicated=False)
        else:
            row.update(status="uncertain", reason="discovery_replication_disagree", replicated=False)


def apply_complementary_suppression_v2(rows: list[dict[str, Any]]) -> None:
    """Withhold M4/M5 margins when a category is below-k.

    The overall count plus the other four category counts would reveal the
    unsupported category by subtraction. Clear both published and audit-only
    sufficient statistics for that category and its overall margin.
    """
    for metric in ("M4", "M5"):
        metric_rows = [row for row in rows if row.get("metric_id") == metric]
        unsupported = [
            row for row in metric_rows
            if row.get("cell") != {"scope": "overall"} and not row.get("k_min_ok")
        ]
        if not unsupported:
            continue
        targets = unsupported + [
            row for row in metric_rows if row.get("cell") == {"scope": "overall"}
        ]
        for row in targets:
            for key in (
                "numerator", "denominator", "baseline_numerator", "baseline_denominator",
                "dx", "dn", "dbx", "dbn", "rx", "rn", "rbx", "rbn", "sx", "sn",
                "discovery_p_value",
            ):
                row[key] = None
            row["snapshot"] = {
                "numerator": None, "denominator": None, "rate": None,
                "suppressed": True, "suppression_reason": "complementary_suppression",
            }
            row["effect"] = None
            row["replication"] = {
                "numerator": None, "denominator": None,
                "baseline_numerator": None, "baseline_denominator": None,
                "effect": None, "adjusted_q": None, "p_value": None,
            }
            row["multiple_testing"]["adjusted_q"] = None
            row["status"] = "uncertain"
            row["reason"] = "complementary_suppression"
            row["replicated"] = False
            row["k_min_ok"] = False
            row["suppressed"] = True
            row["_complementary_suppression"] = True


def m1_discovery_qualified(
    focal_events: int,
    focal_total: int,
    complement_events: int,
    complement_total: int,
    adjusted_q: float | None,
    snapshot_focal_total: int,
    snapshot_complement_total: int,
) -> bool:
    """Apply the frozen adverse M1 effect, support, size and FDR thresholds."""
    if min(focal_events, focal_total, complement_events, complement_total) < 0:
        return False
    if focal_events > focal_total or complement_events > complement_total:
        return False
    if min(snapshot_focal_total, snapshot_complement_total) < 500:
        return False
    if min(focal_events, focal_total - focal_events, complement_events, complement_total - complement_events) < K_MIN:
        return False
    focal_rate = focal_events / focal_total
    complement_rate = complement_events / complement_total
    ratio_ok = complement_rate > 0 and focal_rate / complement_rate >= 1.25
    return bool(
        focal_rate - complement_rate >= 0.05
        and ratio_ok
        and adjusted_q is not None
        and 0 <= adjusted_q <= 0.05
    )


def m1_replication_qualified(
    focal_events: int,
    focal_total: int,
    complement_events: int,
    complement_total: int,
    adjusted_q: float | None,
) -> bool:
    """Require a supported same-direction adverse >=5pp, >=1.25x holdout."""
    return m1_replication_effect_qualified(
        focal_events, focal_total, complement_events, complement_total
    ) and adjusted_q is not None and 0 <= adjusted_q <= 0.05


def m1_replication_effect_qualified(
    focal_events: int,
    focal_total: int,
    complement_events: int,
    complement_total: int,
) -> bool:
    """Check holdout effect and support separately from its FDR gate."""
    if min(focal_events, focal_total, complement_events, complement_total) < 0:
        return False
    if focal_events > focal_total or complement_events > complement_total:
        return False
    if min(focal_events, focal_total - focal_events, complement_events, complement_total - complement_events) < K_MIN:
        return False
    focal_rate = focal_events / focal_total
    complement_rate = complement_events / complement_total
    return bool(
        complement_rate > 0
        and focal_rate - complement_rate >= 0.05
        and focal_rate / complement_rate >= 1.25
    )


def m1_monthly_persistent(months: Iterable[tuple[int, int, int, int]]) -> bool:
    """Check adverse sign in >=80% of the 35 predeclared full months.

    Each tuple is (focal_total, focal_unresolved, complement_total,
    complement_unresolved). Months with fewer than 100 combined eligible rows
    are excluded from the sign denominator, as preregistered.
    """
    values = list(months)
    if len(values) != 35:
        return False
    eligible_signs = []
    for focal_total, focal_unresolved, complement_total, complement_unresolved in values:
        counts = (focal_total, focal_unresolved, complement_total, complement_unresolved)
        if min(counts) < 0 or focal_unresolved > focal_total or complement_unresolved > complement_total:
            return False
        if focal_total + complement_total < 100 or focal_total == 0 or complement_total == 0:
            continue
        eligible_signs.append(
            focal_unresolved / focal_total > complement_unresolved / complement_total
        )
    if not eligible_signs:
        return False
    return sum(eligible_signs) / len(eligible_signs) >= 0.8


def _safe_binary_summary(numerator: int, denominator: int) -> dict[str, Any]:
    if numerator < 0 or denominator < 0 or numerator > denominator:
        raise ValueError("invalid aggregate counts")
    support_ok = numerator >= K_MIN and denominator - numerator >= K_MIN
    return {
        "numerator": numerator if support_ok else None,
        "denominator": denominator if support_ok else None,
        "rate": round(numerator / denominator, 6) if support_ok and denominator else None,
        "suppressed": not support_ok,
        "suppression_reason": None if support_ok else "event_or_nonevent_below_k",
    }


def _safe_count_ratio_summary(numerator: int, denominator: int) -> dict[str, Any]:
    """Apply count disclosure floors to join coverage (not a binary outcome)."""
    if numerator < 0 or denominator < 0 or numerator > denominator:
        raise ValueError("invalid aggregate counts")
    support_ok = numerator >= K_MIN and denominator >= K_MIN
    return {
        "numerator": numerator if support_ok else None,
        "denominator": denominator if support_ok else None,
        "rate": round(numerator / denominator, 6) if support_ok and denominator else None,
        "suppressed": not support_ok,
        "suppression_reason": None if support_ok else "linkage_count_below_k",
    }


def _safe_coverage(value: int, label: str) -> dict[str, Any]:
    supported = value >= K_MIN
    return {
        "value": value if supported else None,
        "suppressed": not supported,
        "suppression_reason": None if supported else f"{label}_below_k",
    }


def validate_source_coverage_v2(value: Any) -> None:
    """Allow only the fixed aggregate coverage contract, never arbitrary values."""
    bank_fields = {
        "contact_rows", "contacts_with_reason", "contacts_with_channel",
        "pqr_valid_status", "pqr_valid_sla_flag", "eligible_csat", "linked_eligible_csat",
    }

    def count_summary(item: Any, allowed_reasons: set[str]) -> bool:
        if not isinstance(item, Mapping) or set(item) != {"value", "suppressed", "suppression_reason"}:
            return False
        count = item["value"]
        if item["suppressed"] is True:
            return count is None and item["suppression_reason"] in allowed_reasons
        return (
            item["suppressed"] is False
            and isinstance(count, int) and not isinstance(count, bool) and count >= K_MIN
            and item["suppression_reason"] is None
        )

    if not isinstance(value, Mapping) or set(value) != {"bank", "digital_actions", "marketing_consent", "e0_linkage"}:
        raise ValueError("source coverage does not match the registered aggregate contract")
    bank = value["bank"]
    if not isinstance(bank, Mapping) or set(bank) != bank_fields or not all(
        count_summary(item, {f"{name}_below_k"}) for name, item in bank.items()
    ):
        raise ValueError("source coverage bank counts violate the aggregate disclosure contract")
    digital = value["digital_actions"]
    if not isinstance(digital, Mapping) or set(digital) != {"transactional_actions", "other_views"}:
        raise ValueError("source coverage digital groups are invalid")
    for summary in digital.values():
        if not isinstance(summary, Mapping) or set(summary) != {
            "numerator", "denominator", "rate", "suppressed", "suppression_reason",
        } or summary.get("suppressed") not in (True, False):
            raise ValueError("source coverage digital summary is invalid")
        if summary["suppressed"] is True:
            if any(summary.get(field) is not None for field in ("numerator", "denominator", "rate")) or summary["suppression_reason"] != "event_or_nonevent_below_k":
                raise ValueError("source coverage digital suppression is invalid")
        else:
            numerator, denominator = summary.get("numerator"), summary.get("denominator")
            if (
                not isinstance(numerator, int) or isinstance(numerator, bool)
                or not isinstance(denominator, int) or isinstance(denominator, bool)
                or denominator < numerator or min(numerator, denominator - numerator) < K_MIN
                or summary.get("suppression_reason") is not None
                or not isinstance(summary.get("rate"), (int, float))
                or abs(summary["rate"] - numerator / denominator) > 0.000001
            ):
                raise ValueError("source coverage declares an invalid or sub-k digital count")
    marketing = value["marketing_consent"]
    e0 = value["e0_linkage"]
    if not isinstance(marketing, Mapping) or set(marketing) != {"total_valid"} or not count_summary(marketing["total_valid"], {"marketing_sends_below_k"}):
        raise ValueError("source coverage marketing count violates the aggregate disclosure contract")
    if not isinstance(e0, Mapping) or set(e0) != {"eligible_cases", "matched_cases"}:
        raise ValueError("source coverage E0 linkage fields are invalid")
    for count in e0.values():
        if count is not None and (not isinstance(count, int) or isinstance(count, bool) or count < K_MIN):
            raise ValueError("source coverage contains an unsuppressed count below k")
    if (e0["eligible_cases"] is None) != (e0["matched_cases"] is None):
        raise ValueError("source coverage E0 linkage counts must both be suppressed or disclosed")
    if e0["eligible_cases"] is not None and e0["matched_cases"] is not None and e0["matched_cases"] > e0["eligible_cases"]:
        raise ValueError("source coverage E0 matched count exceeds eligible cases")


def aggregate_digital_events(rows: Iterable[Mapping[str, Any]]) -> dict[str, Any]:
    """Compute the preregistered action-group Error contrast without joins."""
    counts = {name: [0, 0] for name in _DIGITAL_ACTION_GROUPS}
    for row in rows:
        action = row.get("action")
        if not isinstance(action, str):
            continue
        group = next((name for name, actions in _DIGITAL_ACTION_GROUPS.items() if action in actions), None)
        if group is None:
            continue
        pair = counts[group]
        pair[1] += 1
        pair[0] += int(row.get("event_type") == "Error")
    groups = {name: _safe_binary_summary(*counts[name]) for name in _DIGITAL_ACTION_GROUPS}
    left, right = groups["transactional_actions"], groups["other_views"]
    difference = None
    if left["rate"] is not None and right["rate"] is not None:
        difference = round(left["rate"] - right["rate"], 6)
    return {
        "metric_id": "D1", "type": "descriptive_only", "actionability": "context_only",
        "groups": groups, "difference": difference,
        "multiple_testing": {"method": "not_applicable_preregistered_descriptive", "adjusted_q": None},
        "causal_claim": False,
    }


def aggregate_marketing_consent(
    sends: Iterable[Mapping[str, Any]], customer_consent: Mapping[str, Any],
) -> dict[str, Any]:
    """Join send keys transiently to consent flags and return safe aggregates."""
    consent_by_digest = {
        key if isinstance(key, bytes) else _digest(str(key)): value
        for key, value in customer_consent.items()
    }
    counts = {"all": [0, 0], **{channel: [0, 0] for channel in sorted(_MARKETING_CHANNELS)}}
    for send in sends:
        customer_id = send.get("customer_id")
        channel_value = send.get("send_channel")
        if not isinstance(customer_id, str) or not isinstance(channel_value, str):
            continue
        channel = _fold(channel_value)
        if channel not in _MARKETING_CHANNELS:
            continue
        consent = _flag(consent_by_digest.get(_digest(customer_id)))
        if consent is None:
            continue
        for key in ("all", channel):
            pair = counts[key]
            pair[1] += 1
            pair[0] += int(not consent)
    summaries = {name: _safe_binary_summary(*pair) for name, pair in counts.items()}
    channel_summaries = {key: value for key, value in summaries.items() if key != "all"}
    if any(summary["suppressed"] for summary in channel_summaries.values()):
        # Publishing the total and four channels can reconstruct the fifth.
        channel_summaries = {
            channel: {
                "numerator": None, "denominator": None, "rate": None,
                "suppressed": True,
                "suppression_reason": "complementary_channel_suppression",
            }
            for channel in channel_summaries
        }
    return {
        "metric_id": "R1", "type": "risk", "actionability": "context_only",
        **summaries["all"], "by_channel": channel_summaries,
        "multiple_testing": {"method": "not_applicable_preregistered_descriptive", "adjusted_q": None},
        "causal_claim": False, "legal_conclusion": False,
    }


def summarize_e0_complaint_linkage(total_eligible_cases: int, complaint_ids_found_in_bank: int) -> dict[str, Any]:
    """Summarize exact E0-to-PQR join coverage, never the join keys."""
    if min(total_eligible_cases, complaint_ids_found_in_bank) < 0 or complaint_ids_found_in_bank > total_eligible_cases:
        raise ValueError("invalid E0 linkage counts")
    summary = _safe_count_ratio_summary(complaint_ids_found_in_bank, total_eligible_cases)
    return {
        "metric_id": "L1", "type": "descriptive_only", "actionability": "context_only",
        **summary,
        "multiple_testing": {"method": "not_applicable_preregistered_descriptive", "adjusted_q": None},
    }


def build_v2_payload(
    inferential_entries: list[dict[str, Any]],
    descriptive_entries: list[dict[str, Any]],
    source_coverage: Mapping[str, Any],
) -> dict[str, Any]:
    """Assemble and fail-closed validate the exact preregistered v2 catalog."""
    expected = {
        (metric, tuple(sorted(cell.items())))
        for metric in METRICS_V2
        for cell in planned_cells_v2(metric)
    }
    observed: set[tuple[str, tuple[tuple[str, str], ...]]] = set()
    entries_by_key: dict[tuple[str, tuple[tuple[str, str], ...]], dict[str, Any]] = {}
    if len(inferential_entries) != FAMILY_SIZE_V2:
        raise ValueError("v2 inferential catalog must enumerate all 95 cells")
    for entry in inferential_entries:
        required_fields = {
            "id", "family", "title", "metric_id", "cell", "definition", "numerator",
            "denominator", "snapshot", "cells_explored", "multiple_testing", "status",
            "type", "actionability", "caveats",
        }
        if not required_fields.issubset(entry):
            raise ValueError("v2 inferential entry is missing a required field")
        metric = entry.get("metric_id")
        cell = entry.get("cell")
        key = (metric, tuple(sorted(cell.items()))) if isinstance(cell, dict) else None
        if key not in expected or key in observed:
            raise ValueError("v2 inferential catalog has an invalid or duplicate cell")
        observed.add(key)
        entries_by_key[key] = entry
        if entry.get("cells_explored", {}).get("family_size") != FAMILY_SIZE_V2:
            raise ValueError("v2 inferential entry has incorrect explored family size")
        if entry.get("multiple_testing", {}).get("family_size") != FAMILY_SIZE_V2:
            raise ValueError("v2 inferential entry has incorrect multiplicity family size")
        if entry.get("multiple_testing", {}).get("method") != "two_proportion_z_benjamini_hochberg_fdr":
            raise ValueError("v2 inferential entry has incorrect multiplicity method")
        if metric == "M6" and (
            entry.get("actionability") != "context_only"
            or entry.get("type") != "descriptive_only"
        ):
            raise ValueError("M6 is a dependency control and cannot be actionable")
        if metric == "M6" and entry.get("status") in {"corroborated", "corroborated_descriptive"} and not any(
            isinstance(caveat, str) and "audit discrepancy" in caveat.casefold()
            for caveat in entry.get("caveats", [])
        ):
            raise ValueError("M6 supported result requires an audit discrepancy caveat")
    if observed != expected:
        raise ValueError("v2 inferential catalog does not match its frozen family")

    frozen_controls = {
        ("M2", (('channel', 'phone'),)),
        ("M3", (('channel', 'phone'),)),
        ("M4", (('pqr_category', 'technical'),)),
        ("M5", (('pqr_category', 'technical'),)),
    }
    for key in frozen_controls:
        entry = entries_by_key[key]
        if entry.get("negative_control_status") not in {"audit_consistent", "audit_discrepancy"}:
            raise ValueError("v2 negative control must record audit consistency or discrepancy")
        if entry["negative_control_status"] == "audit_consistent" and (
            entry.get("actionability") != "context_only"
            or entry.get("status") not in {"refuted", "uncertain"}
        ):
            raise ValueError("v2 negative control contradicts its audit-consistent disposition")
        if entry["negative_control_status"] == "audit_discrepancy" and not any(
            "audit discrepancy" in caveat.casefold() for caveat in entry.get("caveats", [])
        ):
            raise ValueError("v2 negative control discrepancy must be disclosed")

    required_context = {"D1", "R1", "L1"}
    if len(descriptive_entries) != len(required_context) or {
        entry.get("metric_id") for entry in descriptive_entries
    } != required_context:
        raise ValueError("v2 catalog must contain exactly the three registered context entries")
    for entry in descriptive_entries:
        required_fields = {
            "id", "family", "title", "metric_id", "cell", "definition", "numerator",
            "denominator", "snapshot", "cells_explored", "multiple_testing", "status",
            "type", "actionability", "caveats",
        }
        if not required_fields.issubset(entry):
            raise ValueError("v2 descriptive entry is missing a required field")
        testing = entry.get("multiple_testing", {})
        if testing.get("method") != "not_applicable_preregistered_descriptive" or testing.get("adjusted_q") is not None:
            raise ValueError("v2 descriptive entry must not claim a hypothesis test")
        if entry.get("actionability") != "context_only":
            raise ValueError("v2 descriptive entries must remain context only")
        expected_type = {"D1": "descriptive_only", "R1": "risk", "L1": "descriptive_only"}[entry["metric_id"]]
        if entry.get("type") != expected_type or entry.get("status") != "context_only":
            raise ValueError("v2 descriptive entry violates its registered semantics")
    e1_entry = entries_by_key[("E1", (('scope', 'overall'),))]
    if e1_entry.get("actionability") != "covered_existing_capability":
        raise ValueError("E1 is already covered by an existing read capability")
    validate_source_coverage_v2(source_coverage)

    payload = {
        "benchmark": "OPBENCH-lite",
        "version": "2",
        "interpretation": "Synthetic hackathon snapshot; descriptive association and split replication only, not causal or production impact evidence.",
        "inputs": {
            "bank_tables": ["call_center_interactions", "complaints", "satisfaction_surveys", "digital_events", "campaign_sends", "customers"],
            "e0_tables": ["case", "copilot_query"],
        },
        "privacy": {"minimum_count": K_MIN, "aggregate_only": True, "row_data_included": False},
        "source_coverage": dict(source_coverage),
        "negative_controls": {
            "entry_refs": ["M2:phone", "M3:phone", "M4:technical", "M5:technical"],
            "agent_outliers": {
                "origin": "audited_source", "status": "no_outlier_evidence",
                "aggregate_statement": AGENT_OUTLIER_CONTROL, "individuals_emitted": False,
            },
        },
        "entries": inferential_entries + descriptive_entries,
    }
    validate_safe_pack(payload)
    return payload


_DEFINITIONS_V2 = {
    "M1": "Unresolved first-contact flag divided by contacts with a valid resolution flag, compared with other reasons in the same channel.",
    "M2": "Complaint-tagged contacts divided by contacts with a recognized reason in the channel.",
    "M3": "Unresolved complaint-tagged contacts divided by contacts with a valid unresolved flag in the channel.",
    "M4": "PQRs with final snapshot status Open, In Process, or Escalated divided by PQRs with recognized status.",
    "M5": "PQRs whose supplied sla_breached flag is true divided by PQRs with a parseable flag; not an independent SLA audit.",
    "M6": "Linked CSAT scores of 1–2 divided by linked valid CSAT scores; dependency control, not a standalone opportunity.",
    "E1": "Cases with the discovery-selected leading query signature divided by all eligible cases in the discovery or holdout partition.",
}


def _stat_entry_v2(row: dict[str, Any], position: int) -> dict[str, Any]:
    metric = row["metric_id"]
    cell = row["cell"]
    if cell == {"scope": "overall"}:
        cell_label = "overall"
    else:
        cell_label = ", ".join(f"{key}={value}" for key, value in sorted(cell.items()))
    title = f"{metric} · {cell_label}"
    status = row["status"]
    if metric == "M1" and status == "corroborated":
        entry_type, actionability = "problem", "candidate"
    elif metric == "E1":
        entry_type, actionability = "descriptive_only", "covered_existing_capability"
    else:
        entry_type, actionability = "descriptive_only", "context_only"
    caveats = [
        "This aggregate is descriptive and does not establish a cause or production impact.",
        "Synthetic hackathon snapshot; source keys and row-level evidence are never emitted.",
    ]
    if metric == "M6" and status in {"corroborated", "corroborated_descriptive"}:
        caveats.append(
            "Audit discrepancy: supported replicated M6 CSAT contrast contradicts the preregistered dependency-control expectation; context only, not an independent opportunity."
        )
    if row.get("_complementary_suppression"):
        caveats.append(
            "Complementary suppression: this margin is withheld to prevent recovery of a below-k category."
        )
    entry = {
        "id": f"{metric}-{position:02d}",
        "family": "contact_resolution" if metric == "M1" else "contact_context" if metric in {"M2", "M3", "M6"} else "pqr_context" if metric in {"M4", "M5"} else "e0_operations",
        "title": title,
        "metric_id": metric,
        "cell": cell,
        "definition": _DEFINITIONS_V2[metric],
        "numerator": row.get("numerator"),
        "denominator": row.get("denominator"),
        "snapshot": row["snapshot"],
        "discovery": {
            "numerator": row.get("numerator"), "denominator": row.get("denominator"),
            "baseline_numerator": row.get("baseline_numerator"),
            "baseline_denominator": row.get("baseline_denominator"),
            "effect": row.get("effect"),
            "adjusted_q": row["multiple_testing"].get("adjusted_q"),
        },
        "effect": row.get("effect"),
        "replication": row.get("replication"),
        "cells_explored": row["cells_explored"],
        "multiple_testing": row["multiple_testing"],
        "status": status,
        "type": entry_type,
        "actionability": actionability,
        "caveats": caveats,
    }
    control_key = (metric, tuple(sorted(cell.items())))
    if control_key in {
        ("M2", (("channel", "phone"),)),
        ("M3", (("channel", "phone"),)),
        ("M4", (("pqr_category", "technical"),)),
        ("M5", (("pqr_category", "technical"),)),
    }:
        consistent = status in {"refuted", "uncertain"}
        entry["negative_control_status"] = "audit_consistent" if consistent else "audit_discrepancy"
        if not consistent:
            entry["caveats"].append("Audit discrepancy: recomputed status differs from the registered negative-control expectation.")
    return entry


def _context_entry_v2(metric: str, aggregate: Mapping[str, Any], title: str, definition: str) -> dict[str, Any]:
    if metric == "D1":
        primary = aggregate["groups"]["transactional_actions"]
        numerator, denominator = primary["numerator"], primary["denominator"]
        snapshot = dict(aggregate)
        entry_type = "descriptive_only"
    else:
        numerator, denominator = aggregate["numerator"], aggregate["denominator"]
        snapshot = dict(aggregate)
        entry_type = "risk" if metric == "R1" else "descriptive_only"
    return {
        "id": metric, "family": "digital_context" if metric == "D1" else "marketing_risk" if metric == "R1" else "e0_source_coverage",
        "title": title, "metric_id": metric, "cell": {"scope": "overall"},
        "definition": definition, "numerator": numerator, "denominator": denominator,
        "snapshot": snapshot, "discovery": None, "effect": None, "replication": None,
        "cells_explored": {"family_size": 0},
        "multiple_testing": {"method": "not_applicable_preregistered_descriptive", "adjusted_q": None},
        "status": "context_only", "type": entry_type, "actionability": "context_only",
        "caveats": ["Preregistered descriptive context only; no causal or customer-level inference."],
    }


def generate_v2_from_rows(
    contacts: Iterable[Mapping[str, Any]],
    pqrs: Iterable[Mapping[str, Any]],
    surveys: Iterable[Mapping[str, Any]],
    digital_events: Iterable[Mapping[str, Any]],
    campaign_sends: Iterable[Mapping[str, Any]],
    customer_consent: Mapping[str, Any],
    e0: Mapping[str, Any],
) -> tuple[dict[str, Any], dict[str, Any]]:
    """Run the complete in-memory v2 pipeline; inputs may be synthetic fixtures."""
    for key in ("discovery", "replication"):
        if not isinstance(e0.get(key), Mapping) or set(e0[key]) != {"selected", "total"}:
            raise ValueError("E0 query aggregate contract is invalid")
    if set(e0) != {"discovery", "replication", "complaint_ids_matched", "eligible_cases"}:
        raise ValueError("E0 aggregate contract is invalid")
    counts = [e0[split][field] for split in ("discovery", "replication") for field in ("selected", "total")]
    linkage_counts = [e0["complaint_ids_matched"], e0["eligible_cases"]]
    if (
        any(not isinstance(value, int) or isinstance(value, bool) or value < 0 for value in counts + linkage_counts)
        or e0["discovery"]["total"] != E0_DISCOVERY_CASES_V2
        or e0["replication"]["total"] != E0_REPLICATION_CASES_V2
        or e0["eligible_cases"] != E0_ELIGIBLE_CASES_V2
        or e0["discovery"]["selected"] > e0["discovery"]["total"]
        or e0["replication"]["selected"] > e0["replication"]["total"]
        or e0["complaint_ids_matched"] > e0["eligible_cases"]
    ):
        raise ValueError("E0 aggregate values do not match the frozen 200/1800/2000 contract")
    metrics, bank_coverage, monthly_persistence = aggregate_bank_v2_rows(contacts, pqrs, surveys)
    rows = assess_m1_v2(metrics["M1"])
    for metric in ("M2", "M3", "M4", "M5", "M6"):
        rows.extend(assess_accumulator(metric, planned_cells_v2(metric), metrics[metric], family_size=FAMILY_SIZE_V2))
    e1 = assess_e1_counts(
        int(e0["discovery"]["selected"]), int(e0["discovery"]["total"]),
        int(e0["replication"]["selected"]), int(e0["replication"]["total"]),
        family_size=FAMILY_SIZE_V2,
    )
    rows.append(e1)
    if len(rows) != FAMILY_SIZE_V2:
        raise ValueError("v2 metric assessment did not enumerate the frozen 95 cells")
    apply_global_tests_v2(rows, monthly_persistence)
    apply_complementary_suppression_v2(rows)
    entries = []
    for metric in METRICS_V2:
        metric_rows = [row for row in rows if row["metric_id"] == metric]
        for index, row in enumerate(metric_rows, start=1):
            for internal_key in ("_snapshot_focal_total", "_snapshot_complement_total", "_discovery_candidate"):
                row.pop(internal_key, None)
            entries.append(_stat_entry_v2(row, index))

    digital = aggregate_digital_events(digital_events)
    marketing = aggregate_marketing_consent(campaign_sends, customer_consent)
    linkage = summarize_e0_complaint_linkage(
        int(e0["eligible_cases"]), int(e0["complaint_ids_matched"])
    )
    descriptive = [
        _context_entry_v2("D1", digital, "Digital transactional-action error concentration", "Error share for registered transaction actions versus other views; no contact linkage."),
        _context_entry_v2("R1", marketing, "Marketing consent risk by send channel", "Share of validly consented-linked sends addressed when the customer flag is false; risk context, not breach conclusion."),
        _context_entry_v2("L1", linkage, "E0 complaint-ID linkage coverage", "Share of eligible E0 cases whose complaint_id matches a bank complaint row."),
    ]
    coverage = {
        "bank": {key: _safe_coverage(value, key) for key, value in bank_coverage.items()},
        "digital_actions": {"transactional_actions": digital["groups"]["transactional_actions"], "other_views": digital["groups"]["other_views"]},
        "marketing_consent": {"total_valid": _safe_coverage(marketing["denominator"] or 0, "marketing_sends")},
        "e0_linkage": {"eligible_cases": linkage["denominator"], "matched_cases": linkage["numerator"]},
    }
    payload = build_v2_payload(entries, descriptive, coverage)
    audit_rows = [dict(row) for row in rows]
    audit = {
        "benchmark": "OPBENCH-lite", "preregistration": "discovery_v2.md",
        "planned_cell_count": FAMILY_SIZE_V2, "cells": audit_rows,
    }
    validate_safe_pack(audit)
    return payload, audit
