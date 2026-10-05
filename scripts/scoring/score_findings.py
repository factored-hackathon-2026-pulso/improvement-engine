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
import sys
import unicodedata

DIRECTION_EPS = 0.005

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
    # M6L / M6R / M6U are variants of M6 (survey low score); the sensor flags depends_on separately.
    if len(m) == 3 and m.startswith("M6") and m[2].isalpha():
        return "M6"
    return m


def norm_cell(dims):
    return tuple(sorted((_strip(k).replace(" ", "_"), norm_value(v)) for k, v in dims.items()))


def direction_of(diff):
    if diff is None:
        return "none"
    return "up" if diff > DIRECTION_EPS else "down" if diff < -DIRECTION_EPS else "none"


def _key(metric, cell, direction):
    return (norm_metric(metric), norm_cell(cell), direction)


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


def _validate(catalog, signals):
    if not isinstance(catalog, dict) or not isinstance(catalog.get("entries"), list):
        raise ScoringError("catalog must be an object with an `entries` list")
    if not isinstance(signals, dict) or not isinstance(signals.get("signals"), list):
        raise ScoringError("signals must be an object with a `signals` list")
    for e in catalog["entries"]:
        if not isinstance(e, dict) or "metric_id" not in e or not isinstance(e.get("cell"), dict):
            raise ScoringError("catalog entry needs metric_id and cell")
    for s in signals["signals"]:
        if not isinstance(s, dict) or "metric" not in s or not isinstance(s.get("dims"), dict):
            raise ScoringError("signal needs metric and dims")


def score(catalog, signals, include_candidate=False, effect_tolerance=0.05, acceptable=None):
    _validate(catalog, signals)
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
        k = _key(e["metric_id"], e["cell"], direction_of(eff))
        if e.get("status") == "corroborated" and e.get("type") == "problem":
            positives[k] = e
        elif e.get("status") == "refuted":
            # a refuted entry is a non-finding for ANY reported direction on that metric + cell
            nonfindings[(k[0], k[1])] = e
    neutral = {(norm_metric(e["metric_id"]), norm_cell(e["cell"])) for e in catalog["entries"]
               if e.get("status") != "refuted" and not (e.get("status") == "corroborated" and e.get("type") == "problem")}
    ok_cells = {(norm_metric(a["metric_id"]), norm_cell(a["cell"])) for a in (acceptable or [])}

    accepted = {"corroborated"} | ({"candidate"} if include_candidate else set())
    reported = [s for s in signals["signals"]
                if s.get("status") in accepted and s.get("direction") in ("up", "down")]

    matched, unmatched_engine, nonfinding_reports, ignored = [], [], [], 0
    seen = set()
    for s in reported:
        k = _key(s["metric"], s["dims"], s["direction"])
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
    return {
        "benchmark": catalog.get("benchmark"),
        "catalog_version": catalog.get("version"),
        "scores": {"recall": recall, "precision": precision, "ranking_agreement_spearman": rank},
        "counts": {"positives": n_pos, "reported": n_rep, "matched": len(matched), "ignored": ignored,
                   "nonfinding_reports": len(nonfinding_reports)},
        "matching": {"key": "metric_id + normalized cell + direction", "include_candidate": include_candidate,
                     "effect_tolerance": effect_tolerance},
        "matched_findings": matched,
        "unmatched_benchmark_findings": [_entry_view(e) for e in positives.values() if e["id"] not in matched_ids],
        "unmatched_engine_findings": unmatched_engine,
        "nonfinding_reports": nonfinding_reports,
        "risk": risk,
        "interpretation": "Association-level agreement with a synthetic benchmark; not causal or production evidence.",
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
