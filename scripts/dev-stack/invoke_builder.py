"""Invoke the pulso-builder task agent with SYNTHETIC input through POST /v1/runs and print ONLY the
end.output_map slots (never tokens, never the evidence). Stdlib only.

    python scripts/dev-stack/invoke_builder.py [--base http://127.0.0.1:8001] [--agent-target <agent id>]
"""
import argparse
import json
import sys
import urllib.error
import urllib.request
import uuid
from pathlib import Path

SYNTHETIC_EVIDENCE = (
    "signal sig-demo-1 verified: reviewers rejected 4 of 5 drafts from the pulso-builder agent because the rationale "
    "field did not name the signal id. Target entity is the prompt p/pulso-builder version 1.0.0. Synthetic data."
)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="http://127.0.0.1:8001")
    ap.add_argument("--state-dir", type=Path, default=Path(__file__).resolve().parents[2] / ".dev-stack")
    ap.add_argument("--agent-target", default="pulso-builder")
    ap.add_argument("--goal", default="Change the prompt p/pulso-builder so that every rationale names the signal id from the evidence.")
    args = ap.parse_args()
    token = json.loads((args.state_dir / "tokens.json").read_text(encoding="utf-8"))["builder"]
    body = {"agent": "pulso-builder", "lang": "es",
            "input": {"agente": args.agent_target, "objetivo": args.goal, "evidencia": SYNTHETIC_EVIDENCE}}
    req = urllib.request.Request(
        args.base + "/v1/runs", data=json.dumps(body).encode(), method="POST",
        headers={"Authorization": "Bearer " + token, "Content-Type": "application/json",
                 "Idempotency-Key": "l3-" + uuid.uuid4().hex})
    try:
        with urllib.request.urlopen(req, timeout=180) as resp:
            status, payload = resp.status, json.loads(resp.read())
    except urllib.error.HTTPError as err:
        status, payload = err.code, json.loads(err.read() or b"{}")
    out = {"http": status, "status": payload.get("status"), "outcome": payload.get("outcome"),
           "output_map": payload.get("output_map") or payload.get("output"),
           "problem": payload.get("code") or payload.get("title")}
    print(json.dumps(out, indent=2))
    return 0 if status == 201 and out["output_map"] else 1


if __name__ == "__main__":
    sys.exit(main())
