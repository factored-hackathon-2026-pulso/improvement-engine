#!/usr/bin/env python3
"""Scheduled probes (PRB1, plan W1-5): run the agent battery (or consume a result JSON), turn confirmed failures into a
`pulso.trigger.v1` request of kind `scheduled` and post it to the engine.

  python scripts/battery/schedule_probes.py --result r.json --state probes-state.json --engine-url http://127.0.0.1:8099 --once
  python scripts/battery/schedule_probes.py --run --base-url http://127.0.0.1:8002 --reps 3 --interval-secs 3600 --out-jsonl t.jsonl

Rules (flake-aware, see probe_cells.py and docs/dev/PROBES.md): a scenario counts when it fails in >= 2 of 3 repetitions
AND in 2 consecutive runs; the same confirmed scenario set is never triggered twice (state `triggered_digest`); an empty
confirmed set resets it, so a recurrence triggers again. A result already consumed (same `finished_at`) is a no-op, so
re-reading a file never counts as a second run.

The request carries ONLY the treated probe cells (`probe_cells`: agent x scenario_family x outcome, labelled `probe`,
counts, `evidence_class: probe_synthetic`; no scenario id, no free text). Transport, CSRF flow (GET /api/v1/auth/session)
and `Idempotency-Key = trigger_key` are the ones of scripts/triggers/agentcore_poller.py (HttpSink / JsonlSink). Credentials:
the engine bearer comes from the process environment (PULSO_ENGINE_TOKEN); nothing is printed.
The engine side also needs the cell table: `--cells-out` writes the ndjson (P1..P3) the value loop reads.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "triggers"))
import probe_cells as pc  # noqa: E402
from agentcore_poller import HttpSink, JsonlSink, SinkError, redact, trigger_key  # noqa: E402

SCHEMA = "pulso.trigger.v1"


def _now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def build_request(*, tenant: str, mission: str, source: str, digest: str, cells: list[dict], at: str, now: float,
                  interval_secs: int) -> dict:
    ref = f"probe:{digest}"
    cd = pc.config_digest()
    key = trigger_key(tenant=tenant, mission=mission, source=source, config_digest=cd, kind="scheduled", ref=ref)
    return {"schema": SCHEMA, "trigger_key": key, "kind": "scheduled", "tenant": tenant, "mission": mission, "source": source,
            "config_digest": cd, "event": {"type": "schedule.tick", "ref": ref, "at": at,
                                           "subject": {"interval_secs": interval_secs, "slot": int(now // interval_secs)}},
            "evidence_class": pc.EVIDENCE_CLASS, "probe_cells": cells, "requested_at": at}


def process(result: dict, state: dict, sink, *, tenant="tenant-local", mission="default", source="agent-battery",
            interval_secs=3600, now: float | None = None) -> dict:
    """One scheduled evaluation. Mutates `state`; the caller saves it. Returns the report (never raises on 'nothing to do')."""
    now = time.time() if now is None else now
    stamp = result.get("finished_at") or result.get("started_at")
    verdicts = pc.scenario_verdicts(result)
    if stamp and stamp == state.get("last_result_at"):
        return {"action": "skip_already_consumed", "signals": [], "rows": []}
    prev = state.get("prev")
    sigs = pc.findings(verdicts, prev)
    confirmed = pc.confirmed_ids(verdicts, prev)
    rows = pc.cell_table(verdicts, prev)
    cells = pc.trigger_cells(sigs)
    digest = pc.set_digest(confirmed)
    rep = {"action": "none", "run_index": state.get("runs", 0) + 1, "confirmed_scenarios": confirmed, "confirmed_digest": digest,
           "flaky": sorted(i for i, v in verdicts.items() if v["flaky"]), "inconclusive": sorted(i for i, v in verdicts.items() if v["status"] == "inconclusive"),
           "signals": sigs, "rows": rows, "counts": pc.combine_reports([], sigs)["probe_synthetic"]["counts"]}
    if confirmed and cells and digest != state.get("triggered_digest"):
        req = build_request(tenant=tenant, mission=mission, source=source, digest=digest, cells=cells, at=_now(), now=now,
                            interval_secs=interval_secs)
        sink.send(req)  # SinkError propagates: state is not advanced, the same key is retried (idempotent)
        state["triggered_digest"] = digest
        rep.update(action="triggered", trigger_key=req["trigger_key"], request=req)
    elif confirmed and digest == state.get("triggered_digest"):
        rep["action"] = "suppressed_unchanged_set"
    if not confirmed:
        state["triggered_digest"] = None
    state["prev"] = {i: {"agent": v["agent"], "family": v["family"], "valid": v["valid"], "failed_reps": v["failed_reps"],
                         "status": v["status"], "flaky": v["flaky"]} for i, v in verdicts.items()}
    state["runs"] = state.get("runs", 0) + 1
    state["last_result_at"] = stamp
    return rep


def run_battery(args) -> dict:
    out = Path(tempfile.mkdtemp(prefix="probes-")) / "result.json"
    cmd = [sys.executable, str(HERE / "run_battery.py"), "run", "--side", "prod", "--reps", str(args.reps), "--out", str(out)]
    if args.base_url:
        cmd += ["--base-url", args.base_url]
    if args.ensure_stack:
        cmd.append("--ensure-stack")
    # env inherited; the runner never prints credentials. Exit 1 = some scenario failed (the point), >1 = the runner broke.
    rc = subprocess.run(cmd, stdout=subprocess.DEVNULL).returncode
    if rc > 1 or not out.exists():
        raise subprocess.CalledProcessError(rc, "run_battery.py")
    return json.loads(out.read_text(encoding="utf-8"))


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    src = ap.add_mutually_exclusive_group(required=True)
    src.add_argument("--result", help="consume this battery result JSON")
    src.add_argument("--run", action="store_true", help="run the battery now (scripts/battery/run_battery.py, side prod)")
    ap.add_argument("--base-url")
    ap.add_argument("--reps", type=int, default=3)
    ap.add_argument("--ensure-stack", action="store_true")
    ap.add_argument("--state", default="probes-state.json")
    ap.add_argument("--engine-url", help="POST to this loopback engine (CSRF + Idempotency-Key)")
    ap.add_argument("--out-jsonl", help="append the request to this JSONL instead")
    ap.add_argument("--cells-out", help="write the cell table ndjson (strict schema) for the engine value loop")
    ap.add_argument("--report-out", help="write the full report JSON (signals, rows, flaky, counts)")
    ap.add_argument("--tenant", default="tenant-local")
    ap.add_argument("--mission", default="default")
    ap.add_argument("--interval-secs", type=int, default=3600)
    ap.add_argument("--once", action="store_true")
    a = ap.parse_args(argv)
    if bool(a.engine_url) == bool(a.out_jsonl):
        print("error: pass exactly one of --engine-url or --out-jsonl", file=sys.stderr)
        return 2
    token = os.environ.get("PULSO_ENGINE_TOKEN")
    sink = HttpSink(a.engine_url, token) if a.engine_url else JsonlSink(Path(a.out_jsonl))
    while True:
        try:
            result = json.loads(Path(a.result).read_text(encoding="utf-8")) if a.result else run_battery(a)
            state = pc.load_state(Path(a.state))
            rep = process(result, state, sink, tenant=a.tenant, mission=a.mission, interval_secs=a.interval_secs)
            pc.save_state(Path(a.state), state)
        except (SinkError, subprocess.CalledProcessError, OSError, ValueError, KeyError) as e:
            print(f"error: {redact(str(e), [token or ''])}", file=sys.stderr)
            return 1
        if a.cells_out and rep["rows"]:
            Path(a.cells_out).write_text(pc.to_ndjson(rep["rows"], strict_schema=True), encoding="utf-8", newline="\n")
        if a.report_out:
            Path(a.report_out).write_text(json.dumps(rep, indent=1, sort_keys=True), encoding="utf-8")
        print(json.dumps({"action": rep["action"], "run_index": rep.get("run_index"), "confirmed": len(rep.get("confirmed_scenarios", [])),
                          "flaky": len(rep.get("flaky", [])), "counts": rep.get("counts")}, sort_keys=True))
        if a.once or a.result:
            return 0
        time.sleep(a.interval_secs)


if __name__ == "__main__":
    raise SystemExit(main())
