"""Probe cells as a detection SOURCE (PRB1, plan W1-5): battery result -> flake-aware verdicts -> treated cell table ->
level-risk-like findings. Standard library only, pure functions (no I/O except `load_state`/`save_state`).

Evidence class. Probe traffic is SYNTHETIC scripted scenarios. Every row, signal and count produced here carries
`evidence_class: probe_synthetic`; `combine_reports` keeps real-data findings and probe findings in separate lists and
separate counts and refuses to mix them. k-anonymity does not apply (no customer is in a cell), so the k rule of the bank
sensor is replaced by an explicit minimum of valid runs (`MIN_VALID_REPS`).

FLAKE RULE (documented in docs/dev/PROBES.md). The Understand model is an LLM: one utterance flips on a few percent of runs.
  1. A scenario FAILS IN A RUN when it fails in >= ceil(2/3 * valid_reps) valid repetitions (>= 2 of 3). Repetitions that
     ended in a transport error (`... http NNN`) are not valid; fewer than 2 valid repetitions = `inconclusive`, never a failure.
  2. A scenario is CONFIRMED when it failed in this run AND in the immediately previous run (2 consecutive runs).
  3. Only confirmed scenarios make a cell `corroborated`; a first-time failure is a `candidate`, a one-of-three failure is
     `flaky` (visible in P3, never a trigger).
  4. `policy_divergence` (human-owned amount-policy finding) is not a failure.

Metrics (the cell table keeps the bank convention "higher is worse", so pass-rate floors are stored as fail-rate ceilings):
  P1  probe_fail_rate_scenarios   scenarios failing in the run / scenarios; by agent x scenario_family   floor: pass >= 0.80
  P2  probe_fail_rate_agent       same, by agent                                                         floor: pass >= 0.90
  P3  probe_fail_rate_reps        failing valid repetitions / valid repetitions; by agent x scenario_family (flake meter,
                                  no floor: descriptive only)

Cell-table row (the schema `steps_cli cells` reads, see scripts/aggregate/bank_cells.py):
  {"metric","dims":{"agent","scenario_family"},"half":"discovery|holdout","numerator","denominator"}
`discovery` = this run, `holdout` = the previous run (the replication slot, as in the bank sensor). `period` is omitted.
The row is rejected by the current Rust cells sensor only because `agent`/`scenario_family` are not in `ALLOWED_DIMS` and
the bank thresholds (k_min 10, min_support 500) do not fit scripted runs: hence the probe findings are computed here with the
same output shape (see `finding_shape`) and the Rust side needs one additive probe path (documented as a follow-up).
"""
from __future__ import annotations

import hashlib
import json
import math
import os
from pathlib import Path

CELL_SCHEMA = "pulso.probe_cell/1"
STATE_SCHEMA = "pulso.probe_state/1"
EVIDENCE_CLASS = "probe_synthetic"
MIN_VALID_REPS = 2

# Pre-registered floors (fixed here, before looking at any result; changing one changes the config digest).
FLOORS = {
    "P1": {"name": "probe_fail_rate_scenarios", "dims": ("agent", "scenario_family"), "pass_floor": 0.80},
    "P2": {"name": "probe_fail_rate_agent", "dims": ("agent",), "pass_floor": 0.90},
    "P3": {"name": "probe_fail_rate_reps", "dims": ("agent", "scenario_family"), "pass_floor": None},
}


def config_digest() -> str:
    raw = json.dumps({"floors": {k: [v["pass_floor"], list(v["dims"])] for k, v in FLOORS.items()}, "min_valid_reps": MIN_VALID_REPS,
                      "rule": "fail>=ceil(2/3*reps);consecutive=2"}, sort_keys=True, separators=(",", ":"))
    return "sha256:" + hashlib.sha256(raw.encode()).hexdigest()


# ------------------------------------------------------------------------------------------------ verdicts (flake rule)

def _infra_error(rep: dict) -> bool:
    """Transport/server errors of the runner (`start http`, `turn http`, `events http`) say nothing about the agent; a
    behavioural error (e.g. `confirm step without a pending confirmation`: the agent never asked) is a failed repetition."""
    return bool(rep.get("error")) and " http " in f" {rep['error']} "


def scenario_verdicts(result: dict) -> dict[str, dict]:
    """scenario id -> {agent, family, valid, failed_reps, status: pass|fail|inconclusive}. Status `fail` = the run-level
    verdict of rule 1; `failed_reps` in 1..needed-1 is reported as `flaky` through `is_flaky`."""
    out = {}
    for s in result["scenarios"]:
        reps = [r for r in s["reps"] if not _infra_error(r)]
        valid, failed = len(reps), sum(1 for r in reps if not r["passed"])
        need = math.ceil(valid * 2 / 3) if valid else 0
        if valid < MIN_VALID_REPS:
            status = "inconclusive"
        else:
            status = "fail" if failed >= need else "pass"
        out[s["id"]] = {"agent": s["agent"], "family": s["family"], "valid": valid, "failed_reps": failed, "status": status,
                        "flaky": status == "pass" and failed > 0}
    return out


def confirmed_ids(now: dict[str, dict], prev: dict[str, dict] | None) -> list[str]:
    """Rule 2: failing now and failing in the immediately previous run."""
    prev = prev or {}
    return sorted(i for i, v in now.items() if v["status"] == "fail" and prev.get(i, {}).get("status") == "fail")


# ------------------------------------------------------------------------------------------------ cell table

def _tally(verdicts: dict[str, dict]) -> dict[tuple, tuple[int, int]]:
    """(metric, dims tuple) -> (numerator, denominator)."""
    t: dict[tuple, list[int]] = {}

    def add(key, num, den):
        c = t.setdefault(key, [0, 0])
        c[0] += num
        c[1] += den
    for v in verdicts.values():
        if v["status"] == "inconclusive":
            continue
        fam = (("agent", v["agent"]), ("scenario_family", v["family"]))
        failed = int(v["status"] == "fail")
        add(("P1", fam), failed, 1)
        add(("P2", (("agent", v["agent"]),)), failed, 1)
        add(("P3", fam), v["failed_reps"], v["valid"])
    return {k: (a, b) for k, (a, b) in t.items()}


def cell_table(now: dict[str, dict], prev: dict[str, dict] | None) -> list[dict]:
    """ndjson rows for the cells sensor schema. Aggregates only: no scenario id, no text."""
    rows = []
    for half, verd in (("discovery", now), ("holdout", prev or {})):
        for (metric, dims), (num, den) in sorted(_tally(verd).items()):
            rows.append({"metric": metric, "dims": dict(dims), "half": half, "numerator": num, "denominator": den,
                         "evidence_class": EVIDENCE_CLASS})
    return rows


def to_ndjson(rows: list[dict], *, strict_schema: bool = False) -> str:
    """`strict_schema=True` drops `evidence_class` for a consumer that rejects unknown fields (the Rust parser does, and
    names the allowed ones); the class is then carried by the file/job label."""
    keep = ("metric", "dims", "half", "numerator", "denominator")
    return "".join(json.dumps({k: r[k] for k in keep} if strict_schema else r, sort_keys=True, separators=(",", ":")) + "\n" for r in rows)


# ------------------------------------------------------------------------------------------------ findings

def _wilson(x: int, n: int) -> tuple[float, float]:
    p, z = x / n, 1.959964
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return max(0.0, c - h), min(1.0, c + h)


def _stage(num: int, den: int, thr: float) -> dict:
    lo, hi = _wilson(num, den)
    rate = num / den
    return {"numerator": num, "denominator": den, "rate": round(rate, 6), "baseline_rate": thr, "diff": round(rate - thr, 6),
            "ci95_low": round(lo, 6), "ci95_high": round(hi, 6)}


def finding_shape(metric: str, dims: dict, status: str, reason: str, disc: dict, hold: dict | None, confirmed: int) -> dict:
    """Same keys as the Rust `level_json` of branch w14 (`type: level_risk`, `class: risk`, `claim: association`) plus the
    probe labels; and the contrast keys (`status`, `direction`, `discovery`, `holdout`) a contrast consumer reads, so the
    same record parses on both. `p` is absent on purpose: scripted runs are not a sample (rule-based, not a significance test)."""
    sig = {"metric": metric, "metric_name": FLOORS[metric]["name"], "type": "level_risk", "class": "risk", "dims": dims,
           "status": status, "reason": reason, "direction": "up", "claim": "association", "discovery": disc,
           "evidence_class": EVIDENCE_CLASS, "label": "probe", "synthetic": True, "k_rule": "not_applicable_synthetic",
           "confirmed_scenarios": confirmed, "pass_floor": FLOORS[metric]["pass_floor"]}
    if hold is not None:
        sig["holdout"] = hold
    return sig


def findings(now: dict[str, dict], prev: dict[str, dict] | None) -> list[dict]:
    """One signal per floor-bearing cell with any failure. statuses: corroborated | candidate | refuted."""
    tn, tp = _tally(now), _tally(prev or {})
    confirmed = set(confirmed_ids(now, prev))
    out = []
    for (metric, dims), (num, den) in sorted(tn.items()):
        floor = FLOORS[metric]["pass_floor"]
        if floor is None or num == 0:
            continue
        thr = round(1 - floor, 6)
        d = dict(dims)
        disc = _stage(num, den, thr)
        pn = tp.get((metric, dims))
        hold = _stage(pn[0], pn[1], thr) if pn else None
        conf_in_cell = sum(1 for i in confirmed if all(now[i][{"agent": "agent", "scenario_family": "family"}[k]] == v for k, v in d.items()))
        if disc["diff"] <= 0:
            status, reason = "refuted", "pass_rate_at_or_above_floor"
        elif hold is None:
            status, reason = "candidate", "no_previous_run"
        elif hold["diff"] <= 0:
            status, reason = "candidate", "previous_run_above_floor"
        elif conf_in_cell == 0:
            status, reason = "candidate", "different_scenarios_failed_in_previous_run"
        else:
            status, reason = "corroborated", "replicated_in_previous_run"
        out.append(finding_shape(metric, d, status, reason, disc, hold, conf_in_cell))
    return out


def normalise_signal(sig: dict) -> dict:
    """Read a signal from EITHER finding type: a contrast signal (`status`, `direction`, `discovery.diff` vs the rest of the
    metric) or a `level_risk` signal (diff vs a pre-registered threshold). Returns the fields a consumer needs, so the probe
    mapping does not depend on whether branch w14 (`level_risk`) is merged. Contrast signals have no `type`."""
    kind = sig.get("type", "contrast")
    d = sig.get("discovery") or {}
    return {"kind": kind, "metric": sig["metric"], "dims": dict(sig.get("dims") or {}), "status": sig.get("status"),
            "baseline_kind": "threshold" if kind == "level_risk" else "rest_of_metric", "rate": d.get("rate"),
            "baseline_rate": d.get("baseline_rate"), "diff": d.get("diff"), "evidence_class": sig.get("evidence_class", "real")}


def combine_reports(real_signals: list[dict], probe_signals: list[dict]) -> dict:
    """Keep probe findings apart from real-data findings. Counts are per evidence class; nothing is pooled."""
    for s in real_signals:
        if s.get("evidence_class", "real") == EVIDENCE_CLASS:
            raise ValueError("a probe signal in the real-data list")
    for s in probe_signals:
        if s.get("evidence_class") != EVIDENCE_CLASS:
            raise ValueError("a non-probe signal in the probe list")

    def count(sigs):
        c: dict[str, int] = {}
        for s in sigs:
            c[s["status"]] = c.get(s["status"], 0) + 1
        return {"total": len(sigs), "by_status": dict(sorted(c.items()))}
    return {"real": {"signals": real_signals, "counts": count(real_signals)},
            "probe_synthetic": {"signals": probe_signals, "counts": count(probe_signals)}}


# ------------------------------------------------------------------------------------------------ trigger payload

def trigger_cells(signals: list[dict]) -> list[dict]:
    """ONLY the treated probe cells of corroborated findings: agent x scenario_family x outcome, labelled `probe`, counts;
    no scenario id, no free text. P2 (agent level) findings carry scenario_family `*`."""
    cells = []
    for s in signals:
        if s["status"] != "corroborated" or s["metric"] == "P3":
            continue
        d = s["discovery"]
        cells.append({"schema": CELL_SCHEMA, "kind": "probe", "label": "probe", "evidence_class": EVIDENCE_CLASS,
                      "metric": s["metric"], "agent": s["dims"]["agent"], "scenario_family": s["dims"].get("scenario_family", "*"),
                      "outcome": "fail", "n_failed": d["numerator"], "n_scenarios": d["denominator"],
                      "confirmed_scenarios": s["confirmed_scenarios"]})
    return sorted(cells, key=lambda c: (c["metric"], c["agent"], c["scenario_family"]))


def set_digest(confirmed: list[str]) -> str:
    return hashlib.sha256(json.dumps(sorted(confirmed)).encode()).hexdigest()[:32]


# ------------------------------------------------------------------------------------------------ persisted state

def load_state(path: Path) -> dict:
    if Path(path).exists():
        s = json.loads(Path(path).read_text(encoding="utf-8"))
        if s.get("schema") == STATE_SCHEMA:
            return s
    return {"schema": STATE_SCHEMA, "prev": None, "triggered_digest": None, "runs": 0, "last_result_at": None}


def save_state(path: Path, state: dict) -> None:
    p = Path(path)
    tmp = p.with_suffix(p.suffix + ".tmp")
    tmp.write_text(json.dumps(state, sort_keys=True), encoding="utf-8")
    os.replace(tmp, p)
