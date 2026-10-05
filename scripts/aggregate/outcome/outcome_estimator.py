"""Descriptive pre/post outcome monitoring over bank-cell NDJSON aggregates."""

from __future__ import annotations

import argparse
import json
import math
import sys
import random
import unicodedata
from collections import Counter, defaultdict
from pathlib import Path
from statistics import NormalDist

CELL_FIELDS = {"metric", "dims", "half", "period", "numerator", "denominator"}
HALVES = ("discovery", "holdout")
METRIC_DIMS = {
    "M1": {"reason_category", "channel"},
    "M2": {"channel"}, "M3": {"channel"},
    "M4": {"category"}, "M5": {"category"},
    "M10": {"reason_category", "channel"},
    "M6": {"reason_category", "channel"},
    "M6R": {"reason_category", "channel"},
    "M6U": {"reason_category", "channel"},
}
CONTROL_DIMENSIONS = ("reason_category", "category", "channel")
T1_SOURCE_TABLES = ("call_center_interactions", "complaints", "satisfaction_surveys")
DESCRIPTIVE_ONLY_METRICS = {"M10"}
WINDOW_MONTHS = 3
MIN_WINDOW_N = 1500
MATERIALITY = 0.02
ALPHA = 0.05


def _valid_period(value: str) -> bool:
    if not isinstance(value, str) or len(value) != 7 or value[4] != "-":
        return False
    year, month = value[:4], value[5:]
    return year.isdigit() and month.isdigit() and 1 <= int(month) <= 12


def _shift_period(value: str, offset: int) -> str:
    year, month = map(int, value.split("-"))
    ordinal = year * 12 + month - 1 + offset
    return f"{ordinal // 12:04d}-{ordinal % 12 + 1:02d}"


def _cell_key(row: dict) -> tuple:
    return row["metric"], tuple(sorted(row["dims"].items()))


def _metric_scope(cell_keys):
    screened = sorted(key for key in cell_keys if key[0] not in DESCRIPTIVE_ONLY_METRICS)
    descriptive = sorted(key for key in cell_keys if key[0] in DESCRIPTIVE_ONLY_METRICS)
    return {
        "screened_metrics": sorted({key[0] for key in screened}),
        "screened_cells": _safe_count(len(screened)),
        "excluded_descriptive_metrics": sorted({key[0] for key in descriptive}),
        "excluded_descriptive_cells": [
            {"metric": metric, "dims": dict(dim_items)}
            for metric, dim_items in descriptive
        ],
    }


_PQR_CATEGORIES = {
    "transactions": "transactions",
    "fees": "fees",
    "technical": "technical",
    "branch": "branch",
    "service": "service",
}


def _normalize_pqr_category(value: str) -> str:
    folded = unicodedata.normalize("NFKD", value.strip().casefold())
    folded = "".join(char for char in folded if not unicodedata.combining(char))
    try:
        return _PQR_CATEGORIES[folded]
    except KeyError as exc:
        raise ValueError("unrecognized PQR category") from exc


def _validate_rows(rows) -> list[dict]:
    from docs.data.opbench.opbench import normalize_channel, normalize_reason

    validated = []
    seen = set()
    seen_source = set()
    by_key = {}
    for index, row in enumerate(rows):
        if not isinstance(row, dict) or set(row) != CELL_FIELDS:
            raise ValueError(f"row {index} does not match the bank_cells NDJSON schema")
        if row["metric"] not in METRIC_DIMS:
            raise ValueError(f"row {index} has an invalid metric")
        dims = row["dims"]
        if not isinstance(dims, dict) or any(
            not isinstance(k, str) or not isinstance(v, str) for k, v in dims.items()
        ):
            raise ValueError(f"row {index} has invalid dimensions")
        if set(dims) != METRIC_DIMS[row["metric"]]:
            raise ValueError(f"row {index} dimensions do not match the safe metric vocabulary")
        if row["half"] not in HALVES:
            raise ValueError(f"row {index} has an invalid replication half")
        if not _valid_period(row["period"]):
            raise ValueError(f"row {index} has an invalid month period")
        numerator, denominator = row["numerator"], row["denominator"]
        if (
            isinstance(numerator, bool)
            or isinstance(denominator, bool)
            or not isinstance(numerator, int)
            or not isinstance(denominator, int)
            or denominator < 10
            or numerator < 0
            or numerator > denominator
        ):
            raise ValueError(f"row {index} has invalid binary counts")
        if (0 < numerator < 10) or (0 < denominator - numerator < 10):
            raise ValueError(f"row {index} violates the upstream k=10 publication rule")
        # Never serialize source category text. Convert labels to the closed
        # OPBENCH vocabulary before any report projection; unknowns collapse
        # into bounded buckets and IDs/free-text cannot pass through.
        source_key = (row["metric"], tuple(sorted(dims.items())), row["half"], row["period"])
        if source_key in seen_source:
            raise ValueError(f"row {index} duplicates a source cell/month")
        seen_source.add(source_key)
        safe_dims = {}
        for name, value in dims.items():
            if name == "reason_category":
                safe_dims[name] = normalize_reason(value)
            elif name == "channel":
                safe_dims[name] = normalize_channel(value)
            else:
                safe_dims[name] = _normalize_pqr_category(value)
        safe_row = dict(row, dims=safe_dims)
        key = (safe_row["metric"], tuple(sorted(safe_dims.items())), safe_row["half"], safe_row["period"])
        if key in seen:
            # Distinct source aliases can normalize to one safe bucket. Merge
            # counts below, then re-apply k before they can be analyzed.
            existing = by_key[key]
            existing["numerator"] += safe_row["numerator"]
            existing["denominator"] += safe_row["denominator"]
            continue
        seen.add(key)
        validated.append(safe_row)
        by_key[key] = safe_row
    for index, row in enumerate(validated):
        numerator, denominator = row["numerator"], row["denominator"]
        if denominator < 10 or (0 < numerator < 10) or (0 < denominator - numerator < 10):
            raise ValueError(f"normalized row {index} violates the upstream k=10 publication rule")
    return validated


def _pick_control_dimension(dims: dict) -> str | None:
    return next((name for name in CONTROL_DIMENSIONS if name in dims), None)


def _half_estimate(series, target_key, pre_months, post_months, control_keys, min_n, z):
    def get_counts(cell_key, months):
        values = []
        for month in months:
            row = series.get((cell_key, month))
            if row is None:
                return None
            values.append(row)
        return sum(r["numerator"] for r in values), sum(r["denominator"] for r in values)

    treated_pre = get_counts(target_key, pre_months)
    treated_post = get_counts(target_key, post_months)
    if treated_pre is None or treated_post is None:
        return {"reason": "incomplete_treated_window"}

    def aggregate_controls(months):
        totals = []
        for month in months:
            monthly = []
            for control_key in control_keys:
                row = series.get((control_key, month))
                if row is None:
                    return None
                monthly.append(row)
            totals.extend(monthly)
        return sum(r["numerator"] for r in totals), sum(r["denominator"] for r in totals)

    control_pre = aggregate_controls(pre_months)
    control_post = aggregate_controls(post_months)
    if control_pre is None or control_post is None:
        return {"reason": "incomplete_published_sibling_control"}

    counts = (treated_pre, treated_post, control_pre, control_post)
    if any(den < min_n for _, den in counts):
        return {"reason": "underpowered_minimum_support"}

    rates = [num / den for num, den in counts]
    effect = (rates[1] - rates[0]) - (rates[3] - rates[2])
    variance = sum(rate * (1.0 - rate) / den for rate, (_, den) in zip(rates, counts))
    standard_error = math.sqrt(max(variance, 0.0))
    lower, upper = effect - z * standard_error, effect + z * standard_error
    return {
        "effect": effect,
        "lower": lower,
        "upper": upper,
        "treated_pre_n": treated_pre[1],
        "treated_post_n": treated_post[1],
        "reason": None,
    }


def estimate_outcomes(
    rows,
    release_period: str,
    *,
    window_months: int = WINDOW_MONTHS,
    min_window_n: int = MIN_WINDOW_N,
    materiality: float = MATERIALITY,
    alpha: float = ALPHA,
) -> dict:
    """Estimate descriptive DID per published cell using same-metric siblings.

    Hash halves are reported as independent replication cohorts, never as
    untreated controls. Incomplete or suppressed comparator windows fail
    closed. No causal interpretation is emitted.
    """
    if not _valid_period(release_period):
        raise ValueError("release_period must be YYYY-MM")
    if (
        isinstance(window_months, bool) or not isinstance(window_months, int) or window_months < 1
        or isinstance(min_window_n, bool) or not isinstance(min_window_n, int) or min_window_n < 10
        or isinstance(materiality, bool) or not isinstance(materiality, (int, float))
        or not math.isfinite(materiality) or not 0 <= materiality < 1
        or isinstance(alpha, bool) or not isinstance(alpha, (int, float))
        or not math.isfinite(alpha) or not 0 < alpha < 1
    ):
        raise ValueError("invalid estimator configuration")
    data = _validate_rows(rows)
    if not data:
        return {
            "release_period": release_period,
            "window_months": window_months,
            "cells": [],
            "summary": {},
        }

    cell_keys = sorted({_cell_key(row) for row in data})
    metric_scope = _metric_scope(cell_keys)
    screened_cell_keys = [
        key for key in cell_keys if key[0] not in DESCRIPTIVE_ONLY_METRICS
    ]
    family_size = max(1, len(screened_cell_keys) * len(HALVES))
    z = NormalDist().inv_cdf(1.0 - alpha / (2.0 * family_size))
    unique_dims = {key: dict(key[1]) for key in cell_keys}
    candidate_groups = defaultdict(list)
    for candidate_key in cell_keys:
        candidate_dims = unique_dims[candidate_key]
        for control_dim in CONTROL_DIMENSIONS:
            if control_dim in candidate_dims:
                rest = tuple(sorted((k, v) for k, v in candidate_dims.items() if k != control_dim))
                candidate_groups[(candidate_key[0], control_dim, rest)].append(candidate_key)
    controls_for = {}
    for target_key in cell_keys:
        dims = unique_dims[target_key]
        control_dim = _pick_control_dimension(dims)
        target_rest = tuple(sorted((k, v) for k, v in dims.items() if k != control_dim)) if control_dim else ()
        siblings = [key for key in candidate_groups.get((target_key[0], control_dim, target_rest), [])
                    if unique_dims[key][control_dim] != dims[control_dim]] if control_dim else []
        controls_for[target_key] = (control_dim, sorted(siblings))

    series_by_half = {half: {} for half in HALVES}
    all_periods = sorted({row["period"] for row in data})
    available = set(all_periods)
    for row in data:
        series_by_half[row["half"]][(_cell_key(row), row["period"])] = row

    pre_months = [_shift_period(release_period, n) for n in range(-window_months, 0)]
    post_months = [_shift_period(release_period, n) for n in range(1, window_months + 1)]
    globally_missing = [month for month in pre_months + post_months if month not in available]
    output = []
    for target_key in cell_keys:
        metric, dim_items = target_key
        dims = dict(dim_items)
        control_dim, control_keys = controls_for[target_key]
        if metric == "M10":
            # M10 is unresolved handled HOURS (floored from seconds), not a
            # count of Bernoulli trials. The generic binomial interval and
            # its improved/worsened/equivalence decisions are not valid here.
            half_results = {
                half: {"reason": "non_bernoulli_metric_descriptive_only"}
                for half in HALVES
            }
        elif globally_missing:
            half_results = {h: {"reason": "incomplete_global_month_window"} for h in HALVES}
        elif not control_keys:
            half_results = {h: {"reason": "no_published_sibling_control"} for h in HALVES}
        else:
            half_results = {}
            for half in HALVES:
                half_results[half] = _half_estimate(
                    series_by_half[half],
                    target_key,
                    pre_months,
                    post_months,
                    control_keys,
                    min_window_n,
                    z,
                )

        reasons = [half_results[half].get("reason") for half in HALVES]
        good = all(reason is None for reason in reasons)
        if good:
            first, second = (half_results[h] for h in HALVES)
            variances = []
            for value in (first, second):
                se = (value["upper"] - value["effect"]) / z if z else 0.0
                variances.append(se * se)
            if all(v > 0 for v in variances):
                weights = [1.0 / v for v in variances]
                effect = sum(
                    value["effect"] * weight
                    for value, weight in zip((first, second), weights)
                ) / sum(weights)
                standard_error = math.sqrt(1.0 / sum(weights))
            else:
                effect = (first["effect"] + second["effect"]) / 2.0
                standard_error = 0.0
            lower, upper = effect - z * standard_error, effect + z * standard_error
            same_improved = all(
                value["effect"] <= -materiality and value["upper"] < -materiality
                for value in (first, second)
            )
            same_worsened = all(
                value["effect"] >= materiality and value["lower"] > materiality
                for value in (first, second)
            )
            equivalent = all(
                value["lower"] >= -materiality and value["upper"] <= materiality
                for value in (first, second)
            )
            if same_improved:
                status, reason = "improved", None
            elif same_worsened:
                status, reason = "worsened", None
            elif equivalent:
                status, reason = "no_detectable_change", None
            else:
                status, reason = "inconclusive", "half_directions_or_intervals_disagree"
            n_pre = first["treated_pre_n"] + second["treated_pre_n"]
            n_post = first["treated_post_n"] + second["treated_post_n"]
        else:
            status = "inconclusive"
            reason = next((item for item in reasons if item), "incomplete_evidence")
            effect = lower = upper = None
            n_pre = n_post = None if metric == "M10" else 0

        result = {
            "metric": metric,
            "dims": dims,
            "inference_scope": (
                "descriptive_only"
                if metric in DESCRIPTIVE_ONLY_METRICS
                else "screened"
            ),
            "effect_pp": None if effect is None else round(effect * 100.0, 6),
            "interval_family_adjusted_pp": None
            if lower is None
            else [round(lower * 100.0, 6), round(upper * 100.0, 6)],
            "n_pre": n_pre,
            "n_post": n_post,
            "control": None if metric in DESCRIPTIVE_ONLY_METRICS else "same_metric_published_siblings",
            "control_dimension": None if metric in DESCRIPTIVE_ONLY_METRICS else control_dim,
            "control_siblings": (
                []
                if metric in DESCRIPTIVE_ONLY_METRICS
                else [unique_dims[key] for key in control_keys]
            ),
            "status": status,
            "reason": reason,
            "uncertainty_method": (
                "not_computed_non_bernoulli_aggregate"
                if metric == "M10"
                else "naive_independent_binomial_not_customer_cluster_adjusted"
            ),
            "interpretation": (
                "descriptive_input_not_inferentially_estimated"
                if metric == "M10"
                else "descriptive_association_not_causation"
            ),
        }
        output.append(result)

    output.sort(key=lambda row: (row["metric"], tuple(sorted(row["dims"].items()))))
    return {
        "release_period": release_period,
        "window_months": window_months,
        "multiplicity": {
            "method": "bonferroni",
            "family_size": family_size,
            "alpha": alpha,
            **metric_scope,
        },
        "cells": output,
        "summary": dict(sorted(Counter(row["status"] for row in output).items())),
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--data-root", required=True, help="local dataset root; never committed")
    parser.add_argument("--out", required=True, help="aggregate report path outside the repository")
    parser.add_argument("--k", type=int, default=10)
    args = parser.parse_args(argv)
    repository_root = Path(__file__).resolve().parents[3]
    data_root = Path(args.data_root).expanduser().resolve()
    requested_target = Path(args.out).expanduser()
    if requested_target.exists() or requested_target.is_symlink():
        raise ValueError("report output already exists; refusing to overwrite")
    target = requested_target.resolve()
    if target == repository_root or repository_root in target.parents:
        raise ValueError("report output must be outside the repository")
    if target == data_root or data_root in target.parents:
        raise ValueError("report output must be outside the data root")
    from scripts.aggregate.bank_cells import build

    rows, stats = build(data_root, k=args.k, tables=T1_SOURCE_TABLES)
    if stats.get("k") != 10:
        raise ValueError("T1 publication is preregistered at k=10")
    report = validate_placebos_and_sensitivity(rows)
    report["source_metrics"] = sorted(stats.get("metrics", {}))
    report["source_periods"] = len({row["period"] for row in rows})
    target.parent.mkdir(parents=True, exist_ok=True)
    try:
        with target.open("x", encoding="utf-8", newline="\n") as stream:
            stream.write(json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n")
    except FileExistsError as exc:
        raise ValueError("report output already exists; refusing to overwrite") from exc
    return 0


def _release_boundaries(rows, window_months):
    periods = sorted({row["period"] for row in rows})
    if not periods:
        return []
    available = set(periods)
    candidates = []
    period = _shift_period(periods[0], window_months)
    last = _shift_period(periods[-1], -window_months)
    while period <= last:
        required = [
            _shift_period(period, offset)
            for offset in range(-window_months, 0)
        ] + [
            _shift_period(period, offset)
            for offset in range(1, window_months + 1)
        ]
        if all(item in available for item in required):
            candidates.append(period)
        period = _shift_period(period, 1)
    return candidates


def _safe_count(value):
    return value if value == 0 or value >= 10 else None


def _support_bin(monthly_observations):
    if monthly_observations < 500:
        return "<500"
    if monthly_observations < 1000:
        return "500-999"
    if monthly_observations < 2000:
        return "1000-1999"
    return ">=2000"


def _inject_reduction(rows, target_key, *, shift_pp, boundary, window_months):
    """Apply a deterministic aggregate-only reduction and preserve k=10 output."""
    post_months = {
        _shift_period(boundary, offset)
        for offset in range(1, window_months + 1)
    }
    copied = [dict(row, dims=dict(row["dims"])) for row in rows]
    rows_by_half = defaultdict(list)
    for row in copied:
        if _cell_key(row) == target_key and row["period"] in post_months:
            rows_by_half[row["half"]].append(row)

    suppressed = set()
    for half in HALVES:
        monthly_rows = sorted(rows_by_half[half], key=lambda row: row["period"])
        if len(monthly_rows) != len(post_months):
            return None
        exact = [row["denominator"] * shift_pp / 100.0 for row in monthly_rows]
        requested = int(math.floor(sum(exact) + 0.5))
        target = min(requested, sum(row["numerator"] for row in monthly_rows))
        reductions = [
            min(math.floor(value), row["numerator"])
            for value, row in zip(exact, monthly_rows)
        ]
        remaining = target - sum(reductions)
        order = sorted(
            range(len(monthly_rows)),
            key=lambda index: (-(exact[index] - reductions[index]), monthly_rows[index]["period"]),
        )
        for index in order:
            if remaining == 0:
                break
            if reductions[index] < monthly_rows[index]["numerator"]:
                reductions[index] += 1
                remaining -= 1
        for index, row in enumerate(monthly_rows):
            if remaining and reductions[index] < row["numerator"]:
                extra = min(remaining, row["numerator"] - reductions[index])
                reductions[index] += extra
                remaining -= extra
        if remaining:
            raise AssertionError("injection allocation failed")
        for row, reduction in zip(monthly_rows, reductions):
            row["numerator"] -= reduction
            complement = row["denominator"] - row["numerator"]
            if (0 < row["numerator"] < 10) or (0 < complement < 10):
                suppressed.add((row["half"], row["period"]))

    return [
        row for row in copied
        if not (_cell_key(row) == target_key and (row["half"], row["period"]) in suppressed)
    ]


def _injected_shift_screen(rows, *, boundary, window_months, injections_pp=(2, 5, 10)):
    data = _validate_rows(rows)
    cell_keys = sorted({_cell_key(row) for row in data})
    metric_scope = _metric_scope(cell_keys)
    screened_cell_keys = [
        key for key in cell_keys if key[0] not in DESCRIPTIVE_ONLY_METRICS
    ]
    months = {
        _shift_period(boundary, offset)
        for offset in range(-window_months, 0)
    } | {
        _shift_period(boundary, offset)
        for offset in range(1, window_months + 1)
    }
    denominators = defaultdict(int)
    for row in data:
        if row["period"] in months and row["metric"] not in DESCRIPTIVE_ONLY_METRICS:
            denominators[(_cell_key(row), row["half"])] += row["denominator"]

    support_for = {
        key: _support_bin(
            sum(denominators[(key, half)] for half in HALVES) / (len(HALVES) * len(months))
        )
        for key in screened_cell_keys
    }
    support_bins = ("<500", "500-999", "1000-1999", ">=2000")
    cell_counts = Counter(support_for.values())
    detected = {support: Counter() for support in support_bins}
    for key in screened_cell_keys:
        support = support_for[key]
        for shift_pp in injections_pp:
            shifted = _inject_reduction(
                data, key, shift_pp=shift_pp, boundary=boundary, window_months=window_months,
            )
            if shifted is None:
                continue
            result = estimate_outcomes(shifted, boundary, window_months=window_months)
            target = next(
                (
                    item for item in result["cells"]
                    if (item["metric"], tuple(sorted(item["dims"].items()))) == key
                ),
                None,
            )
            if target is not None and target["status"] == "improved":
                detected[support][shift_pp] += 1

    by_support_bin = []
    for support in support_bins:
        cell_count = cell_counts[support]
        rates = {
            str(shift): round(detected[support][shift] / cell_count, 6) if cell_count >= 10 else None
            for shift in injections_pp
        }
        threshold = next(
            (shift for shift in injections_pp if rates[str(shift)] is not None and rates[str(shift)] >= 0.8),
            None,
        )
        by_support_bin.append({
            "support_bin": support,
            "screened_cells": _safe_count(cell_count),
            "detection_rate_by_injection": rates,
            "algorithmic_80pct_shift_threshold_pp": threshold if threshold is not None else (
                ">10" if cell_count >= 10 else None
            ),
        })
    return {
        "boundary": boundary,
        "injections_pp": list(injections_pp),
        **metric_scope,
        "support_measure": "mean_published_denominator_per_hash_half_per_month_across_the_six_window_months",
        "method": "fixed_boundary_shift_both_hash_halves_largest_remainder_and_refit_full_estimator",
        "interpretation": "deterministic_aggregate_shift_response_not_statistical_power_or_causal_effect",
        "by_support_bin": by_support_bin,
    }


def validate_placebos_and_sensitivity(rows, *, placebo_draws=1000, seed=20261005):
    """Screen unique temporal placebo windows; avoid unsupported power claims.

    The result contains only aggregate diagnostics and closed dimension labels;
    no input rows or control-cell margins are copied to the report.
    """
    data = _validate_rows(rows)
    if isinstance(placebo_draws, bool) or not isinstance(placebo_draws, int) or placebo_draws < 1:
        raise ValueError("placebo_draws must be positive")
    window = WINDOW_MONTHS
    boundaries = _release_boundaries(data, window)
    if not boundaries:
        raise ValueError("at least six complete months are required for validation")
    rng = random.Random(seed)
    sampled = sorted(rng.sample(boundaries, min(placebo_draws, len(boundaries))))
    cell_keys = sorted({_cell_key(row) for row in data})
    metric_scope = _metric_scope(cell_keys)
    cell_count = len(cell_keys)
    placebo_counts = Counter()
    # Sampling is without replacement, so each selected boundary is evaluated
    # exactly once. Keep the cache explicit to make that cost bound visible.
    placebo_cache = {}
    for release in sampled:
        if release not in placebo_cache:
            result = estimate_outcomes(data, release)
            screened = [row for row in result["cells"] if row["inference_scope"] == "screened"]
            counts = Counter(row["status"] for row in screened)
            any_signal = any(row["status"] in ("improved", "worsened") for row in screened)
            placebo_cache[release] = counts, any_signal
        counts, any_signal = placebo_cache[release]
        placebo_counts.update(counts)
    signal_windows = sum(1 for release in sampled if placebo_cache[release][1])
    nonsignal_windows = len(sampled) - signal_windows
    placebo_summary_publishable = (
        (signal_windows == 0 or signal_windows >= 10)
        and (nonsignal_windows == 0 or nonsignal_windows >= 10)
    )
    empirical_signal_rate = signal_windows / len(sampled)
    placebo = {
        "seed": seed,
        "sampling": "unique_release_windows_without_replacement",
        "evaluated_windows": len(sampled),
        "eligible_release_boundaries": len(boundaries),
        "cells_per_draw": metric_scope["screened_cells"],
        "screened_metrics": metric_scope["screened_metrics"],
        "excluded_descriptive_metrics": metric_scope["excluded_descriptive_metrics"],
        "excluded_descriptive_cells": metric_scope["excluded_descriptive_cells"],
        "windows_with_candidate_signal": (
            _safe_count(signal_windows) if placebo_summary_publishable else None
        ),
        "empirical_candidate_window_rate": (
            round(empirical_signal_rate, 6) if placebo_summary_publishable else None
        ),
        "screen_bound_fraction": 0.05,
        "screen_bound_met": (
            empirical_signal_rate <= 0.05 if placebo_summary_publishable else None
        ),
        "summary_suppressed": not placebo_summary_publishable,
        "interpretation": "finite_horizon_empirical_placebo_signal_frequency_not_calibrated_type_i_error",
        "status_counts": {
            status: _safe_count(count)
            for status, count in sorted(placebo_counts.items())
        },
    }

    analysis_boundary = "2025-01"
    if analysis_boundary not in boundaries:
        raise ValueError("registered analysis boundary 2025-01 is unavailable")
    primary = estimate_outcomes(data, analysis_boundary)
    all_underpowered = [
        {"metric": row["metric"], "dims": row["dims"], "reason": row["reason"]}
        for row in primary["cells"]
        if row["reason"] and "underpowered" in row["reason"]
    ]
    underpowered = all_underpowered if len(all_underpowered) == 0 or len(all_underpowered) >= 10 else None
    return {
        "protocol": "outcome-discovery-v4",
        "interpretation": "exploratory_descriptive_screen_not_causal_or_confirmatory",
        "window_months": window,
        "analysis_boundary": analysis_boundary,
        "analysis": primary,
        "uncertainty_caveat": "event-level binomial intervals are not cluster-adjusted for repeated customers and are not confirmatory",
        "statistical_power_or_mde": "not_estimated_from_aggregate_cells",
        "placebo": placebo,
        "temporal_sensitivity": "not interpreted as a calibrated false-positive rate; windows overlap in time",
        "underpowered_cells": underpowered,
        "registered_cells": _safe_count(cell_count),
        "metric_scope": metric_scope,
        "underpowered_cells_suppressed": underpowered is None,
        "injected_shift_screen": _injected_shift_screen(
            data, boundary=analysis_boundary, window_months=window,
        ),
    }


if __name__ == "__main__":
    sys.exit(main())
