#!/usr/bin/env python3
"""Generates tests/common/golden_flows.json (the data behind the golden FakeCore) from
bridge-contract/examples/flows/*.json.

Usage (repo root):
    python seams/crates/core-client/gen/gen_fakecore.py          # regenerate
    python seams/crates/core-client/gen/gen_fakecore.py --check  # exit 1 when stale

`tests/golden_drift.rs` recomputes `source_sha256` (same normalisation) and fails when the goldens changed without
regeneration. Only what the FakeCore needs is kept: method, path, the JWT claim profile (purpose/tenant_id/job_id),
the Idempotency-Key header when the golden has one, the request body, and the expected status + body.
"""
import hashlib
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[4]
FLOWS = ROOT / "bridge-contract/examples/flows"
OUT = pathlib.Path(__file__).resolve().parents[1] / "tests/common/golden_flows.json"


def source_sha256() -> str:
    h = hashlib.sha256()
    for p in sorted(FLOWS.glob("*.json")):
        h.update(p.name.encode() + b"\0")
        h.update(p.read_bytes().replace(b"\r\n", b"\n"))
        h.update(b"\0")
    return h.hexdigest()


def build() -> dict:
    flows = {}
    for p in sorted(FLOWS.glob("*.json")):
        d = json.loads(p.read_text(encoding="utf-8"))
        steps = {}
        for s in d["steps"]:
            rq, rs = s["request"], s["response"]
            auth = rq.get("auth") or {}
            steps[s["case"]] = {
                "method": rq["method"],
                "path": rq["path"],
                "purpose": auth.get("purpose"),
                "tenant_id": auth.get("tenant_id"),
                "job_id": auth.get("job_id"),
                "idempotency_key": (rq.get("headers") or {}).get("Idempotency-Key"),
                "request_body": rq.get("body"),
                "status": rs["status"],
                "schema": rs.get("schema"),
                "response_body": rs.get("body"),
            }
        flows[d["flow"]] = steps
    return {"generated_by": "gen/gen_fakecore.py", "source_sha256": source_sha256(), "flows": flows}


def render() -> str:
    return json.dumps(build(), indent=1, sort_keys=True) + "\n"


if __name__ == "__main__":
    text = render()
    if "--check" in sys.argv:
        cur = OUT.read_text(encoding="utf-8").replace("\r\n", "\n") if OUT.exists() else ""
        sys.exit(0 if cur == text else 1)
    OUT.write_bytes(text.encode("utf-8"))
    print(f"wrote {OUT.relative_to(ROOT)}")
