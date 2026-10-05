#!/usr/bin/env python3
"""Export the runs of a local agent-core into the saved envelope the T3 reader consumes (cursor chains ending at an
observed EMPTY terminal page, events per run). Local exporter role only (agent-core TEST issuer); the file holds
run-level data (ids, event payloads) and must be written OUTSIDE the repository.

    python scripts/telemetry/export_runs.py --base-url http://127.0.0.1:8061 --label "SYNTHETIC battery" --out <path>
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location("run_battery", REPO / "scripts" / "battery" / "run_battery.py")
rb = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rb)


def pages(http, auth, path: str, first_after, limit: int = 200) -> list[dict]:
    """Walk `{items,next_after}` pages until the server returns an empty one; record each requested cursor."""
    out, after, requested = [], first_after, None
    while True:
        code, body = http.call("GET", f"{path}?limit={limit}&after={after}", auth.exporter())
        if code != 200:
            raise SystemExit(f"export http {code}")
        items = body.get("items") or []
        out.append({"requested_after": requested, "items": items, "next_after": body.get("next_after")})
        if not items:
            return out
        requested = after = body["next_after"]


def reexec() -> None:
    try:
        import testing.fakes.identity  # noqa: F401
        return
    except ImportError:
        pass
    ac = os.environ.get("PULSO_AGENT_CORE_DIR")
    if not ac or os.environ.get("TEL1_REEXEC"):
        sys.exit("needs agent-core on the path: set PULSO_AGENT_CORE_DIR")
    sys.exit(subprocess.run(["uv", "run", "--project", ac, "python", str(Path(__file__).resolve()), *sys.argv[1:]],
                            env={**os.environ, "TEL1_REEXEC": "1", "PYTHONPATH": str(REPO)}).returncode)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", required=True)
    ap.add_argument("--label", required=True, help='must start with RECORDED | SYNTHETIC | LIVE PLATFORM (T3 evidence label)')
    ap.add_argument("--out", required=True, type=Path)
    a = ap.parse_args(argv)
    reexec()
    out = a.out.resolve()
    if REPO in out.parents:
        sys.exit("output must be outside the repository")
    http, auth = rb.Http(a.base_url), rb.DemoAuth()
    run_pages = pages(http, auth, "/v1/export/runs", 0)
    runs = [r for p in run_pages for r in p["items"]]
    events = {r["run_id"]: pages(http, auth, f"/v1/export/runs/{r['run_id']}/events", -1) for r in runs}
    out.write_text(json.dumps({"_label": a.label, "runs": {"pages": run_pages}, "events": events}), encoding="utf-8")
    print(f"exported {len(runs)} runs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
