"""Deterministic, aggregate-only OPBENCH-lite statistics and safety gates."""

from __future__ import annotations

import hashlib
import math
import re
import unicodedata
from dataclasses import dataclass
from typing import Any, Iterable, Mapping


K_MIN = 10
MIN_EFFECT = 0.05
ALPHA = 0.05
REASONS = (
    "complaint", "transactional", "technical", "general_inquiry", "product",
    "account", "card", "loan", "other", "unclassified",
)
CHANNELS = ("phone", "web", "chat", "email", "branch", "mobile_app", "other")
METRIC_DIMENSIONS = {
    "M1": ("reason_category", "channel"),
    "M2": ("channel",),
    "M3": ("channel",),
    "M4": ("pqr_category",),
    "M5": ("pqr_category",),
    "M6": ("reason_category", "channel"),
    "E1": (),
}
METRIC_LABELS = {
    "M1": "contact_unresolved_rate",
    "M2": "complaint_share_of_contacts",
    "M3": "complaint_unresolved_share_of_unresolved",
    "M4": "pqr_open_rate",
    "M5": "pqr_sla_breach_rate",
    "M6": "survey_low_score_rate",
    "E1": "copilot_repeat_rate",
}
_DENIED_KEYS = re.compile(
    r"(^|_)(customer|interaction|complaint|survey|case|query|turn|agent|document|account)_?id($|_)|"
    r"(^|_)(description|transcript|free_text|query_signature|raw_text|question(?:_[0-9]+)?_text|answer|response_text|open_comments|message|comment|content|body)($|_)",
    re.IGNORECASE,
)
_EMAIL = re.compile(r"\b[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}\b")
_UUID = re.compile(r"\b[0-9a-f]{8}-[0-9a-f-]{27,}\b", re.IGNORECASE)
_IDENTIFIER = re.compile(r"\b(?:CUST|INT|CMP|CASE|QRY|SURV)[-_]?[A-Z0-9]*\d[A-Z0-9]*\b", re.IGNORECASE)
_LONG_DIGITS = re.compile(r"(?<!\d)\d{10,}(?!\d)")


@dataclass(frozen=True)
class Observation:
    """A single transient eligible row; never serialized by this module."""

    cell: Mapping[str, str]
    event: bool
    split: str


@dataclass(frozen=True)
class E0Case:
    """Transient case-level leading signature; never serialized."""

    leading_signature: str
    split: str


class MetricAccumulator:
    """O(cells) counters for streaming source rows; stores no row objects."""

    def __init__(self) -> None:
        self.totals = {"discovery": [0, 0], "replication": [0, 0], "overall": [0, 0]}
        self.grouped: dict[tuple[tuple[str, str], ...], dict[str, list[int]]] = {}

    def add(self, cell: Mapping[str, str], event: bool, split: str) -> None:
        if split not in self.totals:
            raise ValueError("observation split must be discovery, replication, or overall")
        self._add_pair(self.totals[split], event)
        key = tuple(sorted(cell.items()))
        by_split = self.grouped.setdefault(key, {
            "discovery": [0, 0], "replication": [0, 0], "overall": [0, 0],
        })
        self._add_pair(by_split[split], event)

    @staticmethod
    def _add_pair(pair: list[int], event: bool) -> None:
        pair[1] += 1
        pair[0] += int(event)


def _fold(value: str) -> str:
    decomposed = unicodedata.normalize("NFKD", value.strip().casefold())
    return "".join(char for char in decomposed if not unicodedata.combining(char))


def normalize_reason(value: str | None) -> str:
    value = _fold(value or "")
    aliases = {
        "queja": "complaint", "reclamo": "complaint", "complaint": "complaint",
        "transaccional": "transactional", "transactional": "transactional",
        "transaction": "transactional", "tecnico": "technical", "technical": "technical",
        "consulta general": "general_inquiry", "consulta_general": "general_inquiry",
        "general inquiry": "general_inquiry", "general_inquiry": "general_inquiry",
        "producto": "product", "product": "product", "cuenta": "account", "account": "account",
        "tarjeta": "card", "card": "card", "prestamo": "loan", "loan": "loan",
        "credit": "loan", "otro": "other", "other": "other",
    }
    return aliases.get(value, "unclassified")


def normalize_channel(value: str | None) -> str:
    value = _fold(value or "")
    aliases = {
        "phone": "phone", "telefono": "phone", "call": "phone", "call center": "phone",
        "web": "web", "website": "web", "chat": "chat", "live chat": "chat",
        "email": "email", "correo": "email", "branch": "branch", "sucursal": "branch",
        "in-person": "branch", "in person": "branch", "mobile app": "mobile_app",
        "mobile_app": "mobile_app", "app": "mobile_app", "ivr": "phone",
    }
    return aliases.get(value, "unclassified" if not value else "other")


def normalize_pqr_category(value: str | None) -> str:
    """Map exact, published category aliases to the closed domain vocabulary."""
    return normalize_reason(value)


def bucket_for_key(prefix: str, key: str) -> str:
    digest = hashlib.sha256((prefix + key).encode("utf-8")).digest()
    return "discovery" if digest[0] & 1 == 0 else "replication"


def planned_cells(metric_id: str) -> list[dict[str, str]]:
    dims = METRIC_DIMENSIONS[metric_id]
    if metric_id == "E1":
        cells = []
    elif dims == ("reason_category", "channel"):
        cells = [
            {"reason_category": reason, "channel": channel}
            for reason in REASONS for channel in CHANNELS
        ]
    elif dims == ("channel",):
        cells = [{"channel": channel} for channel in CHANNELS]
    elif dims == ("pqr_category",):
        cells = [{"pqr_category": category} for category in REASONS]
    else:
        raise ValueError("unsupported metric dimensions")
    if metric_id != "E1":
        cells.append({"scope": "overall"})
    else:
        cells.append({"scope": "overall"})
    return cells


def benjamini_hochberg(p_values: Iterable[float | None]) -> list[float]:
    """Return monotone BH-adjusted q values; None reserves a p=1 family slot."""
    values = [1.0 if value is None else min(1.0, max(0.0, float(value))) for value in p_values]
    m = len(values)
    order = sorted(range(m), key=lambda index: (values[index], index))
    adjusted = [1.0] * m
    running = 1.0
    for reverse_rank, index in enumerate(reversed(order), start=1):
        rank = m - reverse_rank + 1
        running = min(running, values[index] * m / rank)
        adjusted[index] = min(1.0, running)
    return [round(value, 6) for value in adjusted]


def two_proportion_pvalue(success_a: int, total_a: int, success_b: int, total_b: int) -> float:
    if total_a <= 0 or total_b <= 0:
        return 1.0
    pooled = (success_a + success_b) / (total_a + total_b)
    variance = pooled * (1.0 - pooled) * (1.0 / total_a + 1.0 / total_b)
    if variance <= 0:
        return 1.0
    z = ((success_a / total_a) - (success_b / total_b)) / math.sqrt(variance)
    return min(1.0, math.erfc(abs(z) / math.sqrt(2.0)))


def difference_interval(success_a: int, total_a: int, success_b: int, total_b: int) -> tuple[float, float]:
    p_a = success_a / total_a
    p_b = success_b / total_b
    delta = p_a - p_b
    se = math.sqrt(p_a * (1 - p_a) / total_a + p_b * (1 - p_b) / total_b)
    return (round(delta - 1.96 * se, 6), round(delta + 1.96 * se, 6))


def _count(rows: list[Observation]) -> tuple[int, int]:
    return sum(row.event for row in rows), len(rows)


def _supported(success: int, total: int, k: int) -> bool:
    return success >= k and total - success >= k


def _count_fields(success: int, total: int, support_ok: bool) -> tuple[int | None, int | None]:
    return (success, total) if support_ok else (None, None)


def assess_cells(
    metric_id: str,
    cells: list[dict[str, str]],
    rows: list[Observation],
    family_size: int | None = None,
    k: int = K_MIN,
    min_effect: float = MIN_EFFECT,
) -> list[dict[str, Any]]:
    """Compare each fixed cell with its same-metric pooled complement."""
    accumulator = MetricAccumulator()
    for row in rows:
        accumulator.add(row.cell, row.event, row.split)
        accumulator.add(row.cell, row.event, "overall")
    return assess_accumulator(metric_id, cells, accumulator, family_size, k, min_effect)


def assess_accumulator(
    metric_id: str,
    cells: list[dict[str, str]],
    accumulator: MetricAccumulator,
    family_size: int | None = None,
    k: int = K_MIN,
    min_effect: float = MIN_EFFECT,
) -> list[dict[str, Any]]:
    """Assess cell statistics from streaming sufficient statistics."""
    if metric_id not in METRIC_LABELS:
        raise ValueError("unknown metric")
    if family_size is None:
        family_size = len(cells)
    if family_size < len(cells):
        raise ValueError("family size cannot be smaller than the enumerated cells")

    work: list[dict[str, Any]] = []
    for cell in cells:
        overall = cell == {"scope": "overall"}
        key = tuple(sorted(cell.items()))
        by_split = accumulator.grouped.get(key, {"discovery": [0, 0], "replication": [0, 0]})
        dx, dn = by_split["discovery"] if not overall else accumulator.totals["overall"]
        rx, rn = by_split["replication"] if not overall else accumulator.totals["replication"]
        sx, sn = by_split.get("overall", [0, 0]) if not overall else accumulator.totals["overall"]
        dbx, dbn = (accumulator.totals["discovery"][0] - dx, accumulator.totals["discovery"][1] - dn) if not overall else (0, 0)
        rbx, rbn = (accumulator.totals["replication"][0] - rx, accumulator.totals["replication"][1] - rn) if not overall else (0, 0)
        d_ok = _supported(dx, dn, k) if overall else _supported(dx, dn, k) and _supported(dbx, dbn, k)
        r_ok = _supported(rx, rn, k) if overall else _supported(rx, rn, k) and _supported(rbx, rbn, k)
        d_effect = None if overall else dx / dn - dbx / dbn if d_ok else None
        r_effect = None if overall else rx / rn - rbx / rbn if r_ok else None
        d_p = two_proportion_pvalue(dx, dn, dbx, dbn) if d_ok and not overall else None
        work.append({
            "cell": cell,
            "overall": overall,
            "dx": dx, "dn": dn, "dbx": dbx, "dbn": dbn,
            "rx": rx, "rn": rn, "rbx": rbx, "rbn": rbn,
            "sx": sx, "sn": sn,
            "d_ok": d_ok, "r_ok": r_ok,
            "d_effect": d_effect, "r_effect": r_effect, "d_p": d_p,
        })

    if family_size > len(work):
        discovery_ps = [item["d_p"] for item in work] + [None] * (family_size - len(work))
    else:
        discovery_ps = [item["d_p"] for item in work]
    discovery_qs = benjamini_hochberg(discovery_ps)
    for item, q in zip(work, discovery_qs):
        item["d_q"] = q
        item["candidate"] = bool(
            not item["overall"] and item["d_ok"] and item["d_effect"] is not None
            and abs(item["d_effect"]) >= min_effect and q <= ALPHA
        )
        item["candidate_descriptive"] = bool(
            not item["overall"] and item["d_ok"] and item["d_effect"] is not None
            and abs(item["d_effect"]) >= min_effect and q > ALPHA
        )

    candidates = [item for item in work if item["candidate"] or item["candidate_descriptive"]]
    replication_ps = [
        two_proportion_pvalue(item["rx"], item["rn"], item["rbx"], item["rbn"])
        if item["r_ok"] else None
        for item in candidates
    ]
    replication_qs = benjamini_hochberg(replication_ps)
    replication_q_by_id = {id(item): q for item, q in zip(candidates, replication_qs)}

    output = []
    for item in work:
        overall = item["overall"]
        d_effect = item["d_effect"]
        r_effect = item["r_effect"]
        d_q = item["d_q"] if item["d_ok"] else None
        r_q = replication_q_by_id.get(id(item))
        if overall:
            status = "uncertain"
            reason = "overall_reference_not_tested"
        elif not item["d_ok"] or not item["r_ok"]:
            status = "uncertain"
            reason = "k_support_not_met"
        elif item["candidate"] or item["candidate_descriptive"]:
            same_direction = r_effect is not None and d_effect is not None and r_effect * d_effect > 0
            replicates_effect = same_direction and abs(r_effect) >= min_effect
            if not replicates_effect:
                status = "refuted"
                reason = "effect_or_direction_did_not_replicate"
            elif item["candidate"] and r_q is not None and r_q <= ALPHA:
                status = "corroborated"
                reason = "independent_replication_passed"
            else:
                status = "corroborated_descriptive"
                reason = "effect_replicated_without_adjusted_significance"
        elif d_effect is not None and abs(d_effect) < min_effect and r_effect is not None and abs(r_effect) < min_effect:
            status = "refuted"
            reason = "minimum_effect_not_observed"
        else:
            status = "uncertain"
            reason = "discovery_replication_disagree"

        d_support = item["d_ok"]
        r_support = item["r_ok"]
        sx, sn = item["sx"], item["sn"]
        snapshot_support = _supported(sx, sn, K_MIN)
        delta_ci = difference_interval(item["dx"], item["dn"], item["dbx"], item["dbn"]) if d_support and not overall else None
        rep_ci = difference_interval(item["rx"], item["rn"], item["rbx"], item["rbn"]) if r_support and not overall else None
        d_num, d_den = _count_fields(item["dx"], item["dn"], d_support)
        r_num, r_den = _count_fields(item["rx"], item["rn"], r_support)
        baseline_num, baseline_den = _count_fields(item["dbx"], item["dbn"], d_support and not overall)
        rep_base_num, rep_base_den = _count_fields(item["rbx"], item["rbn"], r_support and not overall)
        output.append({
            "metric_id": metric_id,
            "metric": METRIC_LABELS[metric_id],
            "cell": item["cell"],
            "status": status,
            "reason": reason,
            "numerator": d_num,
            "denominator": d_den,
            "snapshot": {
                "numerator": sx if snapshot_support else None,
                "denominator": sn if snapshot_support else None,
                "rate": round(sx / sn, 6) if snapshot_support else None,
                "suppressed": not snapshot_support,
                "suppression_reason": None if snapshot_support else "event_or_nonevent_below_k",
            },
            "baseline_numerator": baseline_num,
            "baseline_denominator": baseline_den,
            "effect": None if delta_ci is None else {
                "difference": round(d_effect, 6),
                "ci95_low": delta_ci[0], "ci95_high": delta_ci[1],
            },
            "multiple_testing": {
                "method": "two_proportion_z_benjamini_hochberg_fdr",
                "family_size": family_size,
                "adjusted_q": d_q,
            },
            "discovery_p_value": item["d_p"],
            "replicated": status in ("corroborated", "corroborated_descriptive"),
            "replication": {
                "numerator": r_num,
                "denominator": r_den,
                "baseline_numerator": rep_base_num,
                "baseline_denominator": rep_base_den,
                "effect": None if rep_ci is None else {
                    "difference": round(r_effect, 6),
                    "ci95_low": rep_ci[0], "ci95_high": rep_ci[1],
                },
                "adjusted_q": r_q,
                "p_value": two_proportion_pvalue(item["rx"], item["rn"], item["rbx"], item["rbn"])
                if item["r_ok"] and not overall else None,
            },
            "k_min_ok": d_support and r_support,
            "cells_explored": {"family_size": family_size},
            "suppressed": not (d_support and r_support),
        })
    return output


def assess_e1(cases: list[E0Case], family_size: int = 181) -> dict[str, Any]:
    """Freeze the discovery modal signature, then test that same signature in replication."""
    discovery: dict[str, int] = {}
    for case in cases:
        if case.split == "discovery":
            discovery[case.leading_signature] = discovery.get(case.leading_signature, 0) + 1
    if not discovery:
        return _e1_empty(family_size, "no_discovery_query_cases")
    selected = min(discovery, key=lambda signature: (-discovery[signature], signature))
    totals = {"discovery": [0, 0], "replication": [0, 0]}
    for case in cases:
        if case.split not in totals:
            raise ValueError("E0 case split must be discovery or replication")
        pair = totals[case.split]
        pair[1] += 1
        pair[0] += int(case.leading_signature == selected)
    dx, dn = totals["discovery"]
    rx, rn = totals["replication"]
    return assess_e1_counts(dx, dn, rx, rn, family_size)


def assess_e1_counts(dx: int, dn: int, rx: int, rn: int, family_size: int = 181) -> dict[str, Any]:
    """Assess the aggregate counts emitted by the isolated E0 Parquet reader."""
    if family_size < 1 or min(dx, dn, rx, rn) < 0 or dx > dn or rx > rn:
        raise ValueError("invalid E1 sufficient statistics")
    support = _supported(dx, dn, K_MIN) and _supported(rx, rn, K_MIN)
    effect = rx / rn - dx / dn if dn and rn else None
    p_value = two_proportion_pvalue(rx, rn, dx, dn) if support else None
    adjusted = benjamini_hochberg([p_value] + [None] * (family_size - 1))[0]
    if not support:
        status, reason = "uncertain", "k_support_not_met"
    elif effect >= MIN_EFFECT and adjusted <= ALPHA:
        status, reason = "corroborated", "modal_signature_increased_in_replication"
    elif effect <= -MIN_EFFECT and adjusted <= ALPHA:
        status, reason = "refuted", "modal_signature_decreased_in_replication"
    elif abs(effect) < MIN_EFFECT:
        status, reason = "corroborated_descriptive", "modal_signature_rate_stable_across_splits"
    else:
        status, reason = "uncertain", "replication_difference_not_significant"
    ci = difference_interval(rx, rn, dx, dn) if support else None
    snapshot_x, snapshot_n = dx + rx, dn + rn
    snapshot_support = _supported(snapshot_x, snapshot_n, K_MIN)
    return {
        "metric_id": "E1", "metric": METRIC_LABELS["E1"],
        "cell": {"scope": "overall"}, "status": status, "reason": reason,
        "numerator": dx if support else None, "denominator": dn if support else None,
        "snapshot": {
            "numerator": snapshot_x if snapshot_support else None,
            "denominator": snapshot_n if snapshot_support else None,
            "rate": round(snapshot_x / snapshot_n, 6) if snapshot_support else None,
            "suppressed": not snapshot_support,
            "suppression_reason": None if snapshot_support else "event_or_nonevent_below_k",
        },
        "effect": None if ci is None else {
            "difference": round(effect, 6), "ci95_low": ci[0], "ci95_high": ci[1],
        },
        "multiple_testing": {
            "method": "two_proportion_z_benjamini_hochberg_fdr",
            "family_size": family_size, "adjusted_q": adjusted if support else None,
        },
        "discovery_p_value": p_value if support else None,
        "replicated": status in ("corroborated", "corroborated_descriptive"),
        "replication": {
            "numerator": rx if support else None, "denominator": rn if support else None,
            "adjusted_q": adjusted if support else None,
            "p_value": p_value if support else None,
            "effect": None if ci is None else {
                "difference": round(effect, 6), "ci95_low": ci[0], "ci95_high": ci[1],
            },
        },
        "k_min_ok": support, "cells_explored": {"family_size": family_size},
        "suppressed": not support,
    }


def _e1_empty(family_size: int, reason: str) -> dict[str, Any]:
    return {
        "metric_id": "E1", "metric": METRIC_LABELS["E1"],
        "cell": {"scope": "overall"}, "status": "uncertain", "reason": reason,
        "numerator": None, "denominator": None, "effect": None,
        "snapshot": {"numerator": None, "denominator": None, "rate": None, "suppressed": True, "suppression_reason": "no_supported_e0_counts"},
        "multiple_testing": {
            "method": "two_proportion_z_benjamini_hochberg_fdr",
            "family_size": family_size, "adjusted_q": None,
        },
        "replicated": False,
        "replication": {"numerator": None, "denominator": None, "adjusted_q": None, "effect": None},
        "k_min_ok": False, "cells_explored": {"family_size": family_size},
        "suppressed": True,
    }


def validate_safe_pack(value: Any) -> None:
    """Reject row-like fields, likely identifiers/free text, and sub-k counts."""
    def visit(node: Any, key: str = "", path: str = "$" ) -> None:
        if isinstance(node, dict):
            for child_key, child_value in node.items():
                if _DENIED_KEYS.search(str(child_key)):
                    raise ValueError(f"identifier or free-text field is not permitted at {path}.{child_key}")
                visit(child_value, str(child_key), f"{path}.{child_key}")
        elif isinstance(node, list):
            for index, child in enumerate(node):
                visit(child, key, f"{path}[{index}]")
        elif isinstance(node, bool) or node is None:
            return
        elif isinstance(node, int):
            if any(token in key.casefold() for token in ("count", "numerator", "denominator", "support")) and node < K_MIN:
                raise ValueError("count below k must be suppressed")
        elif isinstance(node, float):
            return
        elif isinstance(node, str):
            if _EMAIL.search(node) or _UUID.search(node) or _IDENTIFIER.search(node) or _LONG_DIGITS.search(node):
                raise ValueError(f"identifier-like value is not permitted at {path}")
    visit(value)
