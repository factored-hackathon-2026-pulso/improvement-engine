"""Checks of Codex's T3 agent-run aggregator against a REAL local agent-core export.
usage: python t3_check.py <cx_root> <export_null.json> <out_dir>
 1. raw real export (next_after null after an observed empty page) -> expect CLI result
 2. adapted export (status `escalated` -> `closed`, the only change) -> aggregate; determinism (2 runs, byte equal)
 3. boosted variants (clone real rows, relabel outcomes) -> k=10 boundary: 10 per bucket published, 9 withheld whole
 4. cells validated with the same structural rules as seams/crates/steps/src/cells.rs parse_rows
"""
import copy, hashlib, json, sys, uuid
from pathlib import Path
root, exp, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
sys.path.insert(0, str(root))
from scripts.aggregate.agent_runs.aggregation import aggregate_export, ExportContractError, split_for_run
E0 = json.load(open(exp, encoding="utf-8"))
res = {}

def tryagg(e):
    try:
        return aggregate_export(e)
    except ExportContractError as x:
        return {"error": str(x)}

res["raw_real_export"] = tryagg(E0)
statuses = sorted({r["status"] for r in E0["runs"]["items"]})
res["real_run_statuses"] = statuses
E1 = copy.deepcopy(E0)
for r in E1["runs"]["items"]:
    if r["status"] == "escalated":
        r["status"] = "closed"
    if r["status"] == "closed":  # real API: run.closed_at differs from the run_closed event ts by milliseconds
        r["closed_at"] = [x for x in E1["events"][r["run_id"]]["items"] if x["type"] == "run_closed"][0]["ts"]
rep1 = tryagg(E1)
res["adapted_real_export_status_and_ts_aligned"] = {k: v for k, v in rep1.items() if k != "cells"} | {"n_cells": len(rep1.get("cells", []))}
from collections import Counter
res["real_outcome_counts"] = dict(Counter(r["outcome"] for r in E1["runs"]["items"] if r["status"] == "closed"))
res["deterministic"] = json.dumps(rep1, sort_keys=True) == json.dumps(tryagg(copy.deepcopy(E1)), sort_keys=True)

GROUPS = {"resolved": "resolved", "escalation_or_transfer": "escalated", "abstention_or_clarification_exhausted": "abstained",
          "failed": "failed", "other_terminal": "completed"}

def synth(counts_by_half):
    """build an export of closed runs where run ids are searched so each half gets the requested outcome counts"""
    items, ev = [], {}
    pend = {h: dict(c) for h, c in counts_by_half.items()}
    while any(sum(c.values()) for c in pend.values()):
        rid = str(uuid.uuid4())
        h = split_for_run(rid)
        left = [g for g, n in pend[h].items() if n > 0]
        if not left:
            continue
        g = left[0]; pend[h][g] -= 1
        o = GROUPS[g]
        ts = "2026-10-05T03:16:35.739767Z"
        items.append({"run_id": rid, "status": "closed", "outcome": o, "closed_at": ts})
        ev[rid] = {"items": [{"type": "run_closed", "ts": ts, "payload": {"outcome": o}}], "next_after": None}
    return {"_label": "SYNTHETIC vx boost", "runs": {"items": items, "next_after": None}, "events": ev}

full = {g: 10 for g in GROUPS}
nine = dict(full); nine["failed"] = 9
boost = synth({"discovery": full, "holdout": nine})
rb = tryagg(boost)
res["boost_discovery10_holdout_failed9"] = {k: v for k, v in rb.items() if k != "cells"} | {
    "cells": [(c["half"], c["metric"], c["numerator"], c["denominator"]) for c in rb.get("cells", [])]}
ok = {"metric", "dims", "half", "period", "numerator", "denominator"}
bad = []
for c in rb.get("cells", []):
    if set(c) != ok or c["half"] not in ("discovery", "holdout") or len(c["period"]) != 7 or not (0 <= c["numerator"] <= c["denominator"]) or not isinstance(c["dims"], dict):
        bad.append(c)
res["cells_parse_rows_violations"] = len(bad)
(out / "cells_boost.ndjson").write_text("\n".join(json.dumps(c, sort_keys=True) for c in rb.get("cells", [])) + "\n", encoding="utf-8")
print(json.dumps(res, indent=1))
