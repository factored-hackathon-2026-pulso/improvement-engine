"""EVT1 live run: SYNTHETIC platform history -> cell table -> `steps_cli cells_platform` -> which families light up.

    python scripts/aggregate/platform_event_live.py --steps-cli <path to steps_cli.exe> --out <dir outside the repo> [--cases 8000] [--seed 7]

Everything produced is synthetic (platform-sim backbone, planted effects, see platform_event_synth.py) and is written under --out.
The summary compares the corroborated findings with the planted truth (the effects are authored independently of the sensor)."""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import platform_event_cells as pec  # noqa: E402
import platform_event_synth as synth  # noqa: E402


def planted_dims(spec: str) -> dict:
    return dict(kv.split("=") for kv in spec.split(","))


def matches(plant: dict, dims: dict) -> bool:
    return all(dims.get(k) == v for k, v in plant.items())


def summarize(report: dict, labels: dict) -> dict:
    planted = {m: planted_dims(spec) for m, spec in labels["planted_up"].items()}
    sig = [s for s in report["signals"] if s.get("type") != "level_risk"]
    level = [s for s in report["signals"] if s.get("type") == "level_risk"]
    corro = [s for s in sig if s["status"] == "corroborated"]
    hits = {m: [s for s in corro if s["metric"] == m and matches(p, s["dims"])] for m, p in planted.items()}
    false_pos = [s for s in corro if not (s["metric"] in planted and matches(planted[s["metric"]], s["dims"]))]
    lit = sorted({s["metric"] for s in corro})
    return {
        "families_lit": lit,
        "planted_detected": {m: bool(h) for m, h in hits.items()},
        "corroborated": [{"metric": s["metric"], "dims": s["dims"], "rate": s["discovery"]["rate"], "baseline_rate": s["discovery"]["baseline_rate"],
                          "holdout_rate": s["holdout"]["rate"], "p_adj": s["p_adj"], "r2": s["r2"]["status"]} for s in corro],
        "not_planted_but_corroborated": [{"metric": s["metric"], "dims": s["dims"], "rate": s["discovery"]["rate"], "baseline_rate": s["discovery"]["baseline_rate"]} for s in false_pos],
        "level_risk": [{"metric": s["metric"], "status": s["status"], "rate": s["discovery"]["rate"], "ci95": [s["discovery"]["ci95_low"], s["discovery"]["ci95_high"]]} for s in level],
        "status_counts": {st: sum(1 for s in sig if s["status"] == st) for st in sorted({s["status"] for s in sig})},
        "cells_explored": report["cells_explored"],
        "discards": report["discards"],
    }


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--steps-cli", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--cases", type=int, default=8000)
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    events, cases, labels = synth.write(out, seed=a.seed, n_cases=a.cases)
    rows, stats = pec.build(events, cases)
    (out / "cells.ndjson").write_text(pec.to_ndjson(rows), encoding="utf-8", newline="\n")
    (out / "aggregator_stats.json").write_text(json.dumps(stats, indent=2, sort_keys=True), encoding="utf-8", newline="\n")
    proc = subprocess.run([a.steps_cli, "cells_platform"], input=(out / "cells.ndjson").read_bytes(), capture_output=True, check=True)
    report = json.loads(proc.stdout)
    (out / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8", newline="\n")
    summary = summarize(report, labels)
    summary.update({"synthetic": True, "events": len(events), "cases": len(cases), "cell_rows": len(rows), "semantics": proc.stderr.decode().strip()})
    (out / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True), encoding="utf-8", newline="\n")
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
