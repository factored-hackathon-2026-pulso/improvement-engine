#!/usr/bin/env python3
"""TEL1 pipeline: saved run export -> A1..A9 cell table -> Rust `cells_agent_runs` sensor -> agent-run findings.

    python scripts/telemetry/agent_findings.py --input export.json --out-dir <dir outside the repo> [--steps-cli <exe>]

Writes (immutable, outside the checkout) `agent_cells.ndjson` and `agent_findings.json`. Findings carry
`evidence_class: agent_runs` and live in their own lists, never mixed with bank findings. Aggregates only.
"""
from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Any

from scripts.telemetry.agent_signals import (
    PROTOCOL, aggregate_agent_signals, level_findings, render_ndjson, write_immutable_bytes,
)

REPO = Path(__file__).resolve().parents[2]
MAPPING = json.loads((Path(__file__).with_name("agent_signal_mapping.json")).read_text(encoding="utf-8"))


def run_sensor(ndjson: str, steps_cli: str) -> dict[str, Any]:
    if not ndjson.strip():
        return {"cells_explored": 0, "signals": [], "discards": []}
    p = subprocess.run([steps_cli, "cells_agent_runs"], input=ndjson, capture_output=True, text=True, encoding="utf-8")
    if p.returncode != 0:
        raise RuntimeError("cells_agent_runs sensor failed")
    return json.loads(p.stdout)


def map_finding(finding: dict[str, Any]) -> dict[str, Any] | None:
    for row in MAPPING["rows"]:
        m = row["match"]
        if "kind" in m and finding.get("kind") == m["kind"]:
            return row
        if "metric" in m and finding.get("metric") in m["metric"] and "kind" not in finding:
            if "dims_any" in m and not any(d in finding.get("dims", {}) for d in m["dims_any"]):
                continue
            return row
    return None


def build_findings(report: dict[str, Any], sensor: dict[str, Any], synthetic_traffic: bool) -> dict[str, Any]:
    comparative = []
    for s in sensor.get("signals", []):
        if s.get("status") not in {"corroborated", "candidate", "uncertain"} or not s.get("dims"):
            continue
        f = {**s, "evidence_class": "agent_runs", "claim": "association"}
        row = map_finding(f)
        f["mapping"] = {k: row[k] for k in ("id", "hypothesis", "intervention", "guardrail", "caveats")} if row else None
        comparative.append(f)
    levels = []
    for f in level_findings(report["cells"]):
        row = map_finding(f)
        levels.append({**f, "mapping": {k: row[k] for k in ("id", "hypothesis", "intervention", "guardrail", "caveats")} if row else None})
    return {
        "protocol": PROTOCOL, "evidence_class": "agent_runs",
        "traffic": "synthetic_battery_not_real_customers" if synthetic_traffic else "as_labelled_by_export",
        "k": report["k"], "runs_read": report["runs_read"], "cells_published": len(report["cells"]),
        "cells_suppressed": report["suppressed_cells"], "availability": report["availability"],
        "cells_explored_by_sensor": sensor.get("cells_explored", 0), "sensor_discards": sensor.get("discards", []),
        "comparative_findings": comparative, "level_findings": levels, "descriptive": report["descriptive"],
        "mapping_notice": MAPPING["notice"],
    }


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", required=True, type=Path)
    ap.add_argument("--out-dir", required=True, type=Path)
    ap.add_argument("--steps-cli", default=str(Path("D:/cargo-targets/claude-tel1/debug/steps_cli.exe")))
    ap.add_argument("--synthetic-traffic", action="store_true", help="label: the traffic is the synthetic battery")
    a = ap.parse_args(argv)
    export = json.loads(a.input.read_text(encoding="utf-8"))
    report = aggregate_agent_signals(export)
    nd = render_ndjson(report["cells"])
    findings = build_findings(report, run_sensor(nd, a.steps_cli), a.synthetic_traffic)
    write_immutable_bytes(nd.encode("utf-8"), a.out_dir / "agent_cells.ndjson", repo_root=REPO)
    write_immutable_bytes((json.dumps(findings, indent=1, ensure_ascii=False, sort_keys=True) + "\n").encode("utf-8"),
                          a.out_dir / "agent_findings.json", repo_root=REPO)
    print(f"runs={report['runs_read']} cells={len(report['cells'])} suppressed={report['suppressed_cells']} "
          f"comparative={len(findings['comparative_findings'])} level={len(findings['level_findings'])}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
