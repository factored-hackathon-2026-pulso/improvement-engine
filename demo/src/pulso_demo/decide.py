"""Manual approval CLI for `driver --human-mode manual`: `python -m pulso_demo.decide --out demo/out approve|reject`.
Writes the decision file the paused driver is waiting for, bound to the pending request's proposal id and candidate hash."""

from __future__ import annotations

import argparse
import json
import sys
from datetime import UTC, datetime
from pathlib import Path

from pulso_demo.human_flow import DECISION_FILE, REQUEST_FILE


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("decision", choices=["approve", "reject"])
    a = ap.parse_args(argv)
    out = Path(a.out)
    try:
        req = json.loads((out / REQUEST_FILE).read_text("utf-8"))
    except (OSError, ValueError):
        print("no pending human request in", out, file=sys.stderr)
        return 2
    print(f"{a.decision} proposal {req['proposal_id']} candidate_hash {req['candidate_hash']}")
    (out / DECISION_FILE).write_text(json.dumps({"decision": a.decision, "proposal_id": req["proposal_id"], "candidate_hash": req["candidate_hash"],
                                                 "decided_at": datetime.now(UTC).isoformat()}), "utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
