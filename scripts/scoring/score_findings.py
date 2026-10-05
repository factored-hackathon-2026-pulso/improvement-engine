#!/usr/bin/env python3
"""Score the L1 `cells` sensor output against an OPBENCH-lite catalog.

Inputs
  --catalog  opbench-lite.json: {"entries": [{id, type, status, metric_id, cell, effect.difference, ...}]}
  --signals  `steps_cli cells` JSON: {"cells_explored", "signals": [{metric, dims, status, direction,
             discovery?: {diff}, ...}], "discards": [...]}

Matching key: normalized metric_id + normalized cell + direction (up / down / none). The catalog has no
explicit direction, so it is derived from the sign of `effect.difference` (|d| < 0.005 is `none`).

Catalog roles
  positive    status == "corroborated" and type == "problem"        -> recall denominator
  nonfinding  status == "refuted"                                   -> must not be reported
  neutral     anything else (e.g. corroborated_descriptive)         -> ignored by recall and precision

An engine signal is "reported" when its status is `corroborated` (plus `candidate` with
--include-candidate) and its direction is not `none`.

Level risks (W1-4): a signal with `type == "level_risk"` is a LEVEL against a pre-registered threshold, not a vs-rest
contrast. It is matched (metric + cell, direction ignored; a catalog cell {} or {"scope": "overall"} is the metric level)
only against catalog entries of `type == "risk"`: `corroborated` risk entries are risk positives, `refuted` ones are
non-findings. Level risks and risk entries never enter `recall` / `precision` (problem scores); they are scored under the
separate `risk` block (recall over risk positives, unmatched level risks listed, never penalised).

Scores
  recall      matched positives / positives
  precision   matched positives / reported signals (neutral matches and acceptable disagreements excluded)
  ranking     Spearman rank correlation of engine effect vs catalog effect over matched positives
              (None below 2 matches)
Reports name every nonfinding that the engine reported, separately from unknown (unmatched) cells.
Python standard library only; no network, no environment access.
"""
import argparse
import json
import math
from pathlib import Path
import sys
import unicodedata

# Use OPBENCH v2's frozen vocabularies rather than maintaining a second alias
# table in the scorer. Add the sibling module directory so the CLI works from
# any current working directory without installing the repository as a package.
_REPO_ROOT = Path(__file__).resolve().parents[2]
_OPBENCH_DIR = str(_REPO_ROOT / "docs" / "data" / "opbench")
if _OPBENCH_DIR not in sys.path:
    sys.path.insert(0, _OPBENCH_DIR)
from opbench_v2 import (  # noqa: E402
    CHANNELS_V2,
    PQR_CATEGORIES_V2,
    REASONS_V2,
    SURVEY_CHANNELS_V2,
    normalize_channel_v2,
    normalize_pqr_category_v2,
    normalize_reason_v2,
    normalize_survey_channel_v2,
)
from opbench import validate_safe_pack  # noqa: E402

DIRECTION_EPS = 0.005
K_MIN = 10
SIGNAL_FIELDS = {
    "metric", "dims", "status", "reason", "direction", "claim", "discovery", "holdout",
    "r2", "depends_on", "p_adj", "priority", "exploratory_note",
}
SIGNAL_DIMS = {
    "reason_category", "channel", "category", "case_type", "priority", "survey_type",
    "action", "campaign_type", "customer_segment",
}
STAGE_FIELDS = {
    "numerator", "denominator", "rate", "baseline_numerator", "baseline_denominator",
    "baseline_rate", "diff", "p",
}
SIGNAL_REASONS = {
    "not_significant_after_correction", "holdout_unavailable", "holdout_direction_reversed",
    "replicated_in_holdout", "holdout_not_significant", "no_differential",
    # DET1: pooled-support era and the exploratory tier (never `corroborated`)
    "replication_underpowered", "exploratory_discovery_only", "exploratory_replication_underpowered",
    "exploratory_holdout_replicated", "exploratory_holdout_weak",
}
EXPLORATORY_STATUS = "candidate_exploratory"
EXPLORATORY_REASONS = {"exploratory_discovery_only", "exploratory_replication_underpowered",
                       "exploratory_holdout_replicated", "exploratory_holdout_weak"}
DISCARD_KINDS = {
    "k_violation", "holdout_without_discovery", "no_baseline", "below_min_support",
    "favourable_direction", "exploratory_holdout_reversed",
}
SUMMARY_FIELDS = {"semantics", "method", "cells_explored", "signals", "discards"}
METHOD_FIELDS = {
    "test", "multiplicity", "min_ratio", "replication", "secondary_replication", "alpha",
    "min_effect", "min_support", "k_min",
}
# Optional producer additions (level-risk spec, DET1 profile / pooled-support / baseline / family accounting).
METHOD_OPTIONAL_FIELDS = {"level_risk", "profile", "support_basis", "baseline", "families", "exploratory"}
METRICS = {"M1", "M2", "M3", "M4", "M5", "M6", "M6L", "M6R", "M6U", "M7", "M8", "M9", "M10"}
METRIC_DIMS = {
    "M1": {"reason_category", "channel"}, "M2": {"channel"}, "M3": {"channel"},
    "M4": {"category"}, "M5": {"category"}, "M6": {"reason_category", "channel"},
    "M6L": {"reason_category", "channel"}, "M6R": {"reason_category", "channel"},
    "M6U": {"reason_category", "channel"}, "M7": {"action", "channel"},
    "M8": {"campaign_type", "channel"}, "M9": {"channel", "customer_segment"},
    "M10": {"reason_category", "channel"},
}
KNOWN_SURVEY_CHANNELS = {"email", "phone", "telefono", "ivr", "llamada", "app", "mobile app",
                         "mobile_app", "mobileapp", "web", "sms", "other"}
METHOD_STRING_VALUES = {
    "test": "two_proportion_z_pooled_vs_same_channel_excluding_own_reason",
    "replication": "discovery_holdout_hash_split",
    "secondary_replication": "r2_windows_2023-07..2024-12_vs_2025-01..2026-05",
}

# Normalized vocabulary shared with the catalog (OPBENCH-lite): channel and reason_category values.
ALIASES = {
    "queja": "complaint", "quejas": "complaint", "reclamo": "complaint", "complaint": "complaint",
    "phone": "phone", "telefono": "phone", "ivr": "phone", "llamada": "phone",
    "app": "mobile_app", "mobile app": "mobile_app", "mobile_app": "mobile_app", "mobileapp": "mobile_app",
    "web": "web", "email": "email", "correo": "email", "e-mail": "email",
}


class ScoringError(ValueError):
    pass


def _strip(s):
    s = unicodedata.normalize("NFD", str(s).strip())
    return "".join(c for c in s if unicodedata.category(c) != "Mn").lower()


def norm_value(v):
    k = _strip(v)
    return ALIASES.get(k, k.replace(" ", "_"))


def norm_metric(m):
    m = str(m).strip().upper()
    # M6L is the linked-row form of the catalog's M6 estimand. M6R/M6U are
    # different resolved/unresolved estimands and must never collapse to M6.
    if m == "M6L":
        return "M6"
    return m


def _norm_dimension_value(key, value, metric, catalog_version):
    folded = _strip(value)
    if catalog_version != "2":
        return norm_value(folded)
    if key == "reason_category":
        try:
            return normalize_reason_v2(folded)
        except ValueError:
            return folded.replace(" ", "_")
    if key == "channel":
        if norm_metric(metric) == "M6":
            # The v2 survey vocabulary intentionally folds SMS and unknown
            # survey channels into `other`; preserve already-canonical labels.
            if folded in {"email", "phone", "mobile_app", "web", "other"}:
                return folded
            return normalize_survey_channel_v2(folded)
        try:
            return normalize_channel_v2(folded.replace(" ", "_"))
        except ValueError:
            return folded.replace(" ", "_")
    if key in {"category", "pqr_category"}:
        try:
            return normalize_pqr_category_v2(folded)
        except ValueError:
            return folded.replace(" ", "_")
    return folded.replace(" ", "_")


def norm_cell(dims, metric=None, catalog_version="2"):
    return tuple(sorted(
        (_strip(k).replace(" ", "_"), _norm_dimension_value(
            _strip(k).replace(" ", "_"), v, metric, catalog_version))
        for k, v in dims.items()
    ))


def direction_of(diff):
    if diff is None:
        return "none"
    return "up" if diff > DIRECTION_EPS else "down" if diff < -DIRECTION_EPS else "none"


def _key(metric, cell, direction, catalog_version="2"):
    normalized_metric = norm_metric(metric)
    return (normalized_metric, norm_cell(cell, normalized_metric, catalog_version), direction)


def _ranks(xs):
    order = sorted(range(len(xs)), key=lambda i: xs[i])
    r = [0.0] * len(xs)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and xs[order[j + 1]] == xs[order[i]]:
            j += 1
        for t in range(i, j + 1):
            r[order[t]] = (i + j) / 2.0 + 1.0
        i = j + 1
    return r


def spearman(a, b):
    if len(a) < 2:
        return None
    ra, rb = _ranks(a), _ranks(b)
    ma, mb = sum(ra) / len(ra), sum(rb) / len(rb)
    cov = sum((x - ma) * (y - mb) for x, y in zip(ra, rb))
    va = sum((x - ma) ** 2 for x in ra)
    vb = sum((y - mb) ** 2 for y in rb)
    if va == 0 or vb == 0:
        return None
    return round(cov / (va * vb) ** 0.5, 6)


def norm_risk_cell(cell):
    """The metric level: {} and {"scope": "overall"} are the same cell."""
    c = norm_cell(cell)
    return () if c == (("scope", "overall"),) else c


def _entry_view(e):
    return {"id": e.get("id"), "metric_id": e.get("metric_id"), "cell": e.get("cell"),
            "status": e.get("status"), "effect": (e.get("effect") or {}).get("difference")}


def _signal_view(s):
    return {"metric": s.get("metric"), "dims": s.get("dims"), "status": s.get("status"),
            "direction": s.get("direction"), "effect": _engine_effect(s)}


def _engine_effect(s):
    d = s.get("discovery")
    if isinstance(d, dict) and isinstance(d.get("diff"), (int, float)):
        return d["diff"]
    return None


def _validate_signal_stage(stage, label):
    if not isinstance(stage, dict):
        raise ScoringError(f"{label} must be a valid aggregate stage summary")
    if set(stage) != STAGE_FIELDS:
        raise ScoringError(f"{label} must include all producer stage fields")
    for numerator_key, denominator_key in (
        ("numerator", "denominator"), ("baseline_numerator", "baseline_denominator"),
    ):
        numerator, denominator = stage[numerator_key], stage[denominator_key]
        if (isinstance(numerator, bool) or not isinstance(numerator, int)
                or isinstance(denominator, bool) or not isinstance(denominator, int)
                or denominator < K_MIN or numerator < 0 or numerator > denominator
                or numerator not in {0, denominator} and min(numerator, denominator - numerator) < K_MIN):
            raise ScoringError(f"{label} violates the binary privacy floor")
    for field in {"rate", "baseline_rate"} & set(stage):
        value = stage[field]
        if (isinstance(value, bool) or not isinstance(value, (int, float))
                or not math.isfinite(value) or not 0 <= value <= 1):
            raise ScoringError(f"{label}.{field} must be a finite rate between zero and one")
    rounded_rate = round(stage["numerator"] / stage["denominator"], 6)
    if abs(stage["rate"] - rounded_rate) > 1e-6:
        raise ScoringError(f"{label}.rate does not match counts")
    rounded_baseline_rate = round(stage["baseline_numerator"] / stage["baseline_denominator"], 6)
    if abs(stage["baseline_rate"] - rounded_baseline_rate) > 1e-6:
        raise ScoringError(f"{label}.baseline_rate does not match counts")
    for field in {"diff", "p"} & set(stage):
        value = stage[field]
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
            raise ScoringError(f"{label}.{field} must be finite numeric evidence")
        if field == "p" and not 0 <= value <= 1:
            raise ScoringError(f"{label}.p must be between zero and one")
    if abs(stage["diff"] - (stage["rate"] - stage["baseline_rate"])) > 2e-6:
        raise ScoringError(f"{label}.diff does not match rounded rates")


def _validate_signal_summary(signals):
    if not SUMMARY_FIELDS <= set(signals) or set(signals) - SUMMARY_FIELDS - {"level_tests"}:
        raise ScoringError("signal summary must contain exactly the producer summary fields")
    if signals["semantics"] != "claude-standin":
        raise ScoringError("signal semantics is unsupported")
    explored = signals["cells_explored"]
    if isinstance(explored, bool) or not isinstance(explored, int) or explored < 0:
        raise ScoringError("cells_explored must be a non-negative integer")
    method = signals["method"]
    if (not isinstance(method, dict) or not METHOD_FIELDS <= set(method)
            or set(method) - METHOD_FIELDS - METHOD_OPTIONAL_FIELDS):
        raise ScoringError("signal method must match the producer method envelope")
    for key, expected in METHOD_STRING_VALUES.items():
        if method[key] != expected:
            raise ScoringError(f"signal method.{key} is unsupported")
    if not isinstance(method["multiplicity"], str) or method["multiplicity"] not in {
        "benjamini_hochberg_all_explored_cells", "bonferroni_all_explored_cells",
    }:
        raise ScoringError("signal method multiplicity is unsupported")
    for field in ("min_ratio", "alpha", "min_effect"):
        value = method[field]
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
            raise ScoringError(f"signal method.{field} must be finite")
    if method["min_ratio"] <= 0 or not 0 < method["alpha"] < 1 or method["min_effect"] <= 0:
        raise ScoringError("signal method thresholds are invalid")
    for field in ("min_support", "k_min"):
        value = method[field]
        if isinstance(value, bool) or not isinstance(value, int) or value < K_MIN:
            raise ScoringError(f"signal method.{field} is below the supported minimum")
    try:
        validate_safe_pack({"method": method})
    except ValueError as error:
        raise ScoringError(f"signal method failed privacy validation: {error}") from error
    if "discards" in signals:
        if not isinstance(signals["discards"], list):
            raise ScoringError("signal discards must be a list")
        for discard in signals["discards"]:
            if not isinstance(discard, dict) or not isinstance(discard.get("kind"), str):
                raise ScoringError("each discard must have a named kind")
            if discard["kind"] not in DISCARD_KINDS:
                raise ScoringError("discard kind is unsupported")
            keys = set(discard)
            if keys == {"kind", "count"}:
                count = discard["count"]
                if isinstance(count, bool) or not isinstance(count, int) or count < K_MIN:
                    raise ScoringError("discard count below the privacy floor must be suppressed")
            elif keys == {"kind", "count", "suppressed"}:
                if discard["count"] is not None or discard["suppressed"] is not True:
                    raise ScoringError("suppressed discard count must be null with suppressed=true")
            else:
                raise ScoringError("discard summary has an invalid shape")
            try:
                validate_safe_pack(discard)
            except ValueError as error:
                raise ScoringError(f"discard summary failed privacy validation: {error}") from error

    holdout_candidates = sum(
        isinstance(signal.get("reason"), str) and signal.get("reason") in {
            "holdout_unavailable", "holdout_direction_reversed", "replicated_in_holdout",
            "holdout_not_significant",
        }
        for signal in signals["signals"] if isinstance(signal, dict)
    )
    holdout_candidates = max(1, holdout_candidates)
    for signal in signals["signals"]:
        if not isinstance(signal, dict) or "metric" not in signal or not isinstance(signal.get("dims"), dict):
            raise ScoringError("signal needs metric and dims")
        if signal.get("type") == "level_risk":
            continue  # W1-4: a level against a pre-registered threshold has its own shape; matched only to risk entries in score()
        required_signal_fields = {"metric", "dims", "status", "reason", "direction", "claim"}
        if required_signal_fields - set(signal):
            raise ScoringError("signal is missing required producer fields")
        if set(signal) - SIGNAL_FIELDS:
            raise ScoringError("signal contains unknown signal field")
        if not isinstance(signal["metric"], str) or not signal["metric"].strip():
            raise ScoringError("signal metric must be a non-empty string")
        if signal["metric"] not in METRICS:
            raise ScoringError("signal metric is unsupported")
        try:
            validate_safe_pack({
                key: signal[key] for key in ("metric", "dims", "reason", "claim", "depends_on")
                if key in signal
            })
        except ValueError as error:
            raise ScoringError(f"signal failed privacy validation: {error}") from error
        if any(not isinstance(key, str) or key not in SIGNAL_DIMS or not isinstance(value, str)
               for key, value in signal["dims"].items()):
            raise ScoringError("signal dimensions must use closed aggregate keys and string values")
        metric, dims = signal["metric"], signal["dims"]
        if not dims:
            if (signal.get("status") != "refuted" or signal.get("reason") != "no_differential"
                    or signal.get("direction") != "none"
                    or signal.get("claim") != "association"
                    or set(signal) != {"metric", "dims", "status", "reason", "direction", "claim"}):
                raise ScoringError("no_differential aggregate shape is invalid")
            continue
        if set(dims) != METRIC_DIMS[metric] and not (metric == "M1" and set(dims) == {"reason_category"}):
            raise ScoringError("signal dimensions do not match metric")
        for key, value in dims.items():
            folded = _strip(value)
            if key == "reason_category":
                try:
                    normalized = normalize_reason_v2(folded)
                except ValueError as error:
                    raise ScoringError("dimension value is unsupported") from error
                valid_values = set(REASONS_V2)
            elif key == "channel":
                if metric in {"M6", "M6L", "M6R", "M6U"}:
                    if folded not in KNOWN_SURVEY_CHANNELS:
                        raise ScoringError("dimension value is unsupported")
                    normalized = folded if folded in SURVEY_CHANNELS_V2 else normalize_survey_channel_v2(folded)
                    valid_values = set(SURVEY_CHANNELS_V2)
                else:
                    canonical = norm_value(folded).replace(" ", "_")
                    if canonical in CHANNELS_V2:
                        normalized = canonical
                    else:
                        try:
                            normalized = normalize_channel_v2(canonical)
                        except ValueError as error:
                            raise ScoringError("dimension value is unsupported") from error
                    valid_values = set(CHANNELS_V2)
            elif key == "category":
                try:
                    normalized = normalize_pqr_category_v2(folded)
                except ValueError as error:
                    raise ScoringError("dimension value is unsupported") from error
                valid_values = set(PQR_CATEGORIES_V2)
            else:
                raise ScoringError(f"{metric} dimension vocabulary is not documented")
            if normalized not in valid_values:
                raise ScoringError("dimension value is unsupported")
        if "status" in signal and (
            not isinstance(signal["status"], str)
            or signal["status"] not in {"candidate", "corroborated", "refuted", "uncertain", EXPLORATORY_STATUS}
        ):
            raise ScoringError("signal status is unsupported")
        if "direction" in signal and (
            not isinstance(signal["direction"], str)
            or signal["direction"] not in {"up", "down", "none"}
        ):
            raise ScoringError("signal direction is unsupported")
        if "claim" in signal and signal["claim"] != "association":
            raise ScoringError("signal claim must remain an association")
        if "reason" in signal and not isinstance(signal["reason"], str):
            raise ScoringError("signal reason must be a string")
        if "reason" in signal and signal["reason"] not in SIGNAL_REASONS:
            raise ScoringError("signal reason is unsupported")
        if "p_adj" in signal:
            p_adj = signal["p_adj"]
            if (isinstance(p_adj, bool) or not isinstance(p_adj, (int, float))
                    or not math.isfinite(p_adj) or not 0 <= p_adj <= 1):
                raise ScoringError("signal p_adj must be a probability between zero and one")
        signal_keys = set(signal)
        base_keys = {"metric", "dims", "status", "reason", "direction", "claim"}
        if signal["direction"] != "up":
            raise ScoringError("cell signal direction is unsupported")
        if signal["reason"] == "not_significant_after_correction":
            if signal["status"] != "uncertain" or signal_keys != base_keys | {"discovery"}:
                raise ScoringError("signal status requires discovery-only evidence")
        else:
            status_by_reason = {
                "holdout_unavailable": "candidate",
                "holdout_direction_reversed": "refuted",
                "replicated_in_holdout": "corroborated",
                "holdout_not_significant": "uncertain",
                "replication_underpowered": "uncertain",
                **{r: EXPLORATORY_STATUS for r in EXPLORATORY_REASONS},
            }
            expected_status = status_by_reason.get(signal["reason"])
            if (expected_status is None or signal["status"] != expected_status
                    or not {"discovery", "p_adj", "r2"}.issubset(signal_keys)):
                raise ScoringError("signal status requires holdout evidence")
            if signal["reason"] in {"holdout_unavailable", "exploratory_discovery_only"}:
                if "holdout" in signal:
                    raise ScoringError("candidate without holdout has an invalid shape")
            elif "holdout" not in signal:
                raise ScoringError("signal status requires holdout evidence")
            if signal_keys - (base_keys | {"discovery", "holdout", "p_adj", "r2", "depends_on", "priority", "exploratory_note"}):
                raise ScoringError("signal status has an invalid producer shape")
        for stage_name in ("discovery", "holdout"):
            if stage_name in signal:
                _validate_signal_stage(signal[stage_name], f"signal.{stage_name}")
        if signal["reason"] in {
            "holdout_direction_reversed", "replicated_in_holdout", "holdout_not_significant",
        }:
            holdout = signal["holdout"]
            replicates = (holdout["p"] * holdout_candidates < 0.05
                          and holdout["diff"] >= method["min_effect"] / 2.0)
            expected_reason = (
                "holdout_direction_reversed" if holdout["diff"] <= 0.0 else
                "replicated_in_holdout" if replicates else "holdout_not_significant"
            )
            if signal["reason"] != expected_reason:
                raise ScoringError("holdout reason contradicts evidence")
        if "r2" in signal:
            r2 = signal["r2"]
            if not isinstance(r2, dict) or set(r2) - {"status", "w1", "w2"}:
                raise ScoringError("signal.r2 must be a valid two-window summary")
            if (not isinstance(r2.get("status"), str)
                    or r2["status"] not in {"replicated", "not_replicated", "reversed", "not_evaluated"}):
                raise ScoringError("signal.r2 status is unsupported")
            for window in ("w1", "w2"):
                if window in r2:
                    _validate_signal_stage(r2[window], f"signal.r2.{window}")
            has_windows = "w1" in r2 and "w2" in r2
            if (r2["status"] == "not_evaluated" and ("w1" in r2 or "w2" in r2)
                    or r2["status"] != "not_evaluated" and not has_windows):
                raise ScoringError("signal.r2 windows do not match producer status")
            if has_windows:
                w1, w2 = r2["w1"], r2["w2"]
                both_replicate = all(
                    window["diff"] >= method["min_effect"] / 2.0 and window["p"] < 0.05
                    for window in (w1, w2)
                )
                expected_r2 = (
                    "replicated" if both_replicate else
                    "reversed" if w1["diff"] <= 0.0 or w2["diff"] <= 0.0 else "not_replicated"
                )
                if r2["status"] != expected_r2:
                    raise ScoringError("R2 status contradicts windows")
        if "depends_on" in signal and (
            not isinstance(signal["depends_on"], str) or signal["depends_on"] not in METRICS
        ):
            raise ScoringError("signal dependency metric is unsupported")


def _validate(catalog, signals):
    if not isinstance(catalog, dict) or not isinstance(catalog.get("entries"), list):
        raise ScoringError("catalog must be an object with an `entries` list")
    if catalog.get("benchmark") != "OPBENCH-lite" or catalog.get("version") not in {"1.0.0", "2"}:
        raise ScoringError("unsupported catalog version")
    if not isinstance(signals, dict) or not isinstance(signals.get("signals"), list):
        raise ScoringError("signals must be an object with a `signals` list")
    _validate_signal_summary(signals)
    for e in catalog["entries"]:
        if not isinstance(e, dict) or "metric_id" not in e or not isinstance(e.get("cell"), dict):
            raise ScoringError("catalog entry needs metric_id and cell")


def score(catalog, signals, include_candidate=False, effect_tolerance=0.05, acceptable=None):
    _validate(catalog, signals)
    catalog_version = catalog["version"]
    risk_pos, risk_non = {}, {}
    for e in catalog["entries"]:
        if e.get("type") == "risk":
            rk = (norm_metric(e["metric_id"]), norm_risk_cell(e["cell"]))
            if e.get("status") == "corroborated":
                risk_pos[rk] = e
            elif e.get("status") == "refuted":
                risk_non[rk] = e
    catalog = dict(catalog, entries=[e for e in catalog["entries"] if e.get("type") != "risk"])
    level_signals = [s for s in signals["signals"] if s.get("type") == "level_risk"]
    signals = dict(signals, signals=[s for s in signals["signals"] if s.get("type") != "level_risk"])
    positives, nonfindings = {}, {}
    for e in catalog["entries"]:
        eff = (e.get("effect") or {}).get("difference")
        k = _key(e["metric_id"], e["cell"], direction_of(eff), catalog_version)
        if e.get("status") == "corroborated" and e.get("type") == "problem":
            positives[k] = e
        elif e.get("status") == "refuted":
            # a refuted entry is a non-finding for ANY reported direction on that metric + cell
            nonfindings[(k[0], k[1])] = e
    neutral = {(norm_metric(e["metric_id"]), norm_cell(e["cell"], e["metric_id"], catalog_version)
                ) for e in catalog["entries"]
               if e.get("status") != "refuted" and not (e.get("status") == "corroborated" and e.get("type") == "problem")}
    ok_cells = {(norm_metric(a["metric_id"]), norm_cell(a["cell"], a["metric_id"], catalog_version)
                 ) for a in (acceptable or [])}

    accepted = {"corroborated"} | ({"candidate"} if include_candidate else set())
    expl_signals = [s for s in signals["signals"] if s.get("status") == EXPLORATORY_STATUS and s.get("direction") == "up"]
    reported = [s for s in signals["signals"]
                if s.get("status") in accepted and s.get("direction") in ("up", "down")]

    matched, unmatched_engine, nonfinding_reports, ignored = [], [], [], 0
    seen = set()
    for s in reported:
        k = _key(s["metric"], s["dims"], s["direction"], catalog_version)
        mc = (k[0], k[1])
        if k in positives and k not in seen:
            seen.add(k)
            e = positives[k]
            ee, ce = _engine_effect(s), (e.get("effect") or {}).get("difference")
            matched.append({"id": e["id"], "metric_id": e["metric_id"], "cell": e["cell"], "direction": k[2],
                            "engine_effect": ee, "catalog_effect": ce,
                            "effect_within_tolerance": ee is not None and ce is not None
                            and abs(ee - ce) <= effect_tolerance})
        elif mc in nonfindings:
            e = nonfindings[mc]
            nonfinding_reports.append({"id": e["id"], "metric_id": e["metric_id"], "cell": e["cell"],
                                       "engine": _signal_view(s)})
        elif mc in ok_cells or mc in neutral:
            ignored += 1
        elif k in seen:
            ignored += 1  # duplicate of an already matched positive
        else:
            unmatched_engine.append(_signal_view(s))

    risk_matched, risk_unmatched, risk_nonfindings = [], [], []
    for s in level_signals:
        if s.get("status") not in accepted:
            continue
        rk = (norm_metric(s["metric"]), norm_risk_cell(s["dims"]))
        if rk in risk_pos and rk not in {(norm_metric(m["metric_id"]), norm_risk_cell(m["cell"])) for m in risk_matched}:
            e = risk_pos[rk]
            risk_matched.append({"id": e["id"], "metric_id": e["metric_id"], "cell": e["cell"], "engine": _signal_view(s)})
        elif rk in risk_non:
            e = risk_non[rk]
            risk_nonfindings.append({"id": e["id"], "metric_id": e["metric_id"], "cell": e["cell"], "engine": _signal_view(s)})
        elif rk not in risk_pos:
            risk_unmatched.append(_signal_view(s))
    nonfinding_reports.extend(risk_nonfindings)
    rm_ids = {m["id"] for m in risk_matched}
    risk = {
        "positives": len(risk_pos), "reported": len(risk_matched) + len(risk_unmatched) + len(risk_nonfindings),
        "matched": len(risk_matched),
        "recall": None if not risk_pos else round(len(risk_matched) / len(risk_pos), 6),
        "matched_risks": risk_matched,
        "unmatched_benchmark_risks": [_entry_view(e) for e in risk_pos.values() if e["id"] not in rm_ids],
        "unmatched_level_risks": risk_unmatched,
        "matching": "metric_id + normalized cell (type risk entries only; direction ignored)",
    }
    n_pos, n_rep = len(positives), len(reported) - ignored
    recall = None if n_pos == 0 else round(len(matched) / n_pos, 6)
    precision = None if n_rep <= 0 else round(len(matched) / n_rep, 6)
    pairs = [(m["engine_effect"], m["catalog_effect"]) for m in matched
             if m["engine_effect"] is not None and m["catalog_effect"] is not None]
    rank = spearman([p[0] for p in pairs], [p[1] for p in pairs])
    matched_ids = {m["id"] for m in matched}
    # Exploratory tier (DET1), scored SEPARATELY: never part of recall / precision above. Unlabelled = not in the catalog; it is
    # neither a finding nor a false positive (the regression proof is the quality gate).
    expl_matched, expl_unlabelled, expl_nonfinding, expl_ignored = [], [], [], 0
    expl_seen = set(seen)
    for s in expl_signals:
        k = _key(s["metric"], s["dims"], s["direction"], catalog_version)
        mc = (k[0], k[1])
        if k in positives and k not in expl_seen:
            expl_seen.add(k)
            expl_matched.append(positives[k]["id"])
        elif k in positives:
            expl_ignored += 1
        elif mc in nonfindings:
            expl_nonfinding.append(_signal_view(s))
        elif mc in ok_cells or mc in neutral:
            expl_ignored += 1
        else:
            expl_unlabelled.append(_signal_view(s))
    all_matched = matched_ids | set(expl_matched)
    exploratory = {
        "signals": len(expl_signals), "newly_matched_positives": len(expl_matched),
        "unlabelled": len(expl_unlabelled), "matched_nonfindings": len(expl_nonfinding), "ignored_neutral_or_duplicate": expl_ignored,
        "recall_strict_plus_exploratory": None if n_pos == 0 else round(len(all_matched) / n_pos, 6),
        "unlabelled_cells": expl_unlabelled, "nonfinding_cells": expl_nonfinding,
        "note": "exploratory: weaker statistical evidence; unlabelled cells are not findings and not false positives",
    }
    return {
        "benchmark": catalog.get("benchmark"),
        "catalog_version": catalog.get("version"),
        "scores": {"recall": recall, "precision": precision, "ranking_agreement_spearman": rank},
        "counts": {"positives": n_pos, "reported": n_rep, "matched": len(matched), "ignored": ignored,
                   "nonfinding_reports": len(nonfinding_reports)},
        "matching": {"key": "metric_id + versioned normalized cell + direction", "include_candidate": include_candidate,
                     "effect_tolerance": effect_tolerance},
        "matched_findings": matched,
        "unmatched_benchmark_findings": [_entry_view(e) for e in positives.values() if e["id"] not in matched_ids],
        "unmatched_engine_findings": unmatched_engine,
        "nonfinding_reports": nonfinding_reports,
        "risk": risk,
        "exploratory": exploratory,
        "interpretation": "Agreement with a derived OPBENCH-lite catalog; not independent accuracy, causal evidence, or production performance.",
        "validation_limitations": (
            "The catalog is a method reference, not independent ground truth. For v2, the bank-cell sensor and "
            "catalog use different customer splits and statistical gates on the same source snapshot; interpret "
            "scores as cross-protocol agreement, not independent accuracy."
            if catalog_version == "2" else
            "The catalog is a method reference, not independent ground truth; scores are agreement with that catalog."
        ),
    }


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--catalog", required=True)
    ap.add_argument("--signals", required=True)
    ap.add_argument("--out")
    ap.add_argument("--acceptable", help="JSON list of {metric_id, cell} disagreements accepted by review")
    ap.add_argument("--include-candidate", action="store_true")
    ap.add_argument("--effect-tolerance", type=float, default=0.05)
    a = ap.parse_args(argv)
    try:
        with open(a.catalog, encoding="utf-8") as f:
            cat = json.load(f)
        with open(a.signals, encoding="utf-8") as f:
            sig = json.load(f)
        acc = None
        if a.acceptable:
            with open(a.acceptable, encoding="utf-8") as f:
                acc = json.load(f)
        res = score(cat, sig, a.include_candidate, a.effect_tolerance, acc)
    except (OSError, ValueError) as e:
        print(f"score_findings: {e}", file=sys.stderr)
        return 2
    text = json.dumps(res, indent=2, sort_keys=True)
    if a.out:
        with open(a.out, "w", encoding="utf-8") as f:
            f.write(text + "\n")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
