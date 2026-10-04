"""DEMO-0 entry point (python side of demo/run-demo0.ps1).

Replays the ten-step E2E-THREAD-01 (no containers) through the GT0 collector, writes the engine-run report and prints a
human-readable summary that lists the doubles[] (what is NOT real) FIRST and then the ten steps with their labels.
The summary REFUSES a report that fails the G1 check() or lacks a plan double: a clean summary never hides a stand-in.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
GOV = ROOT / "scripts" / "gov"
PLAN_DOUBLES = ("model", "jev", "issuer", "product", "host", "gate", "data.origin")


class Demo0Refused(Exception):
    """The report is not honest enough to summarise."""


def _engine_run():
    sys.path.insert(0, str(GOV))
    import gt0_gate
    return gt0_gate.engine_run()


def summarize(report: dict) -> str:
    violations = _engine_run().check(report)
    if violations:
        raise Demo0Refused("G1 check() failed: " + "; ".join(f"{v['rule']} {v['where']}: {v['msg']}" for v in violations))
    doubles = report.get("doubles") or []
    parts = {d.get("part") for d in doubles}
    missing = [p for p in PLAN_DOUBLES if p not in parts]
    if missing:
        raise Demo0Refused("doubles[] lacks " + ", ".join(missing))
    L = [f"DEMO-0 engine-run report: label {report.get('label')}, host {report.get('host')}, target {report.get('target')}, "
         f"contract {report.get('contract_revision')}, sha {str(report.get('sha'))[:12]}",
         f"quality_claims: {report.get('quality_claims')}; gate judge {(report.get('gate') or {}).get('judge')}, "
         f"gate verdict {(report.get('gate') or {}).get('verdict')}", "",
         "NOT REAL (doubles[], listed first; everything below is only as real as this list allows):"]
    for d in doubles:
        extra = ", ".join(f"{k}={d[k]}" for k in ("provider", "data_class") if d.get(k))
        L.append(f"- {d.get('part')}: {d.get('status')}" + (f" ({extra})" if extra else ""))
    for o in report.get("overrides") or []:
        L.append(f"- override: {o.get('label')} of {o.get('of')} verdict {o.get('verdict')} by {o.get('by')} (simulated={o.get('simulated')}): {o.get('reason')}")
    L += ["", "STEPS (id, status, data class, receipt provider):"]
    for s in report.get("steps") or []:
        L.append(f"{s.get('n', '')}. {s['id']}: {s['status']} | {s.get('data_class')} | {(s.get('receipt') or {}).get('provider', '-')}")
    return "\n".join(L) + "\n"


def live_note() -> str:
    return ("LIVE roleplay window: run per roleplay-llm/RUNBOOK.md and e2e-core/THREAD01.md ('Live roleplay window'): "
            "python -m claude_standin.thread01 --live <queue> --workdir <dir> --summary <json> with one fresh-context responder "
            "subagent per request. This script does not spawn models or subagents; log the window in docs/reports/gates/gt0/live-windows.json.")


def realcore_note() -> str:
    return ("REAL-CORE steps (5, 6, 8, 9): run e2e-core/run.ps1 (-PytestArgs '-k','steps_5_6_8_9 and [1]'): one stack on pulso-dev "
            "(Podman, real_local profile), always torn down (try/finally) unless -Keep; one window per fresh stack because the "
            "registry is immutable. This script does not start containers.")


def run_replay(out_dir: Path) -> dict:
    sys.path.insert(0, str(GOV))
    import gt0_collect
    rc = gt0_collect.main(["--out-dir", str(out_dir), "--override-reason", "simulated human override of the failed structural stand-in gate (DEMO-0)"])
    if rc != 0:
        raise Demo0Refused(f"replay collector exited {rc}")
    return json.loads((Path(out_dir) / "report.json").read_text(encoding="utf-8"))


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out-dir", default=str(ROOT / "demo" / "out" / "demo0"))
    ap.add_argument("--from-report")
    ap.add_argument("--live", action="store_true")
    ap.add_argument("--real-core", action="store_true")
    a = ap.parse_args(argv)
    try:
        if a.from_report:
            report = json.loads(Path(a.from_report).read_text(encoding="utf-8"))
        else:
            report = run_replay(Path(a.out_dir))
            print(f"engine-run report written to {Path(a.out_dir) / 'report.json'}\n")
        print(summarize(report))
    except Demo0Refused as e:
        print(f"DEMO-0 REFUSED: {e}", file=sys.stderr)
        return 1
    if a.live:
        print(live_note())
    if a.real_core:
        print(realcore_note())
    return 0


if __name__ == "__main__":
    sys.exit(main())
