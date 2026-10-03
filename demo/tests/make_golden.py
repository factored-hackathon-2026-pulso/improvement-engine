"""Regenerates tests/golden/*.json (python tests/make_golden.py). Analysis is computed from the dataset; the Core section is a fixed,
hand-written sample of what the driver extracts from the real stack (shapes only, values are not real runs)."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from pulso_demo import analysis, bundle, dataset, translate  # noqa: E402

G = Path(__file__).resolve().parent / "golden"


def main() -> None:
    conn = dataset.build()
    sc = analysis.scout(conn)
    ver = analysis.verify(conn, sc["hypotheses"])
    c1 = analysis.first_candidate()
    j1 = analysis.judge(conn, c1)
    c2 = analysis.revise(conn, c1, j1)
    j2 = analysis.judge(conn, c2)
    attempts = [{"candidate": j["candidate"], "improvement": j["improvement"], "native_proxy": j["native_proxy"]} for j in (j1, j2)]
    ok = {"state": "terminal_ok", "core_run_id": "run-sample", "release_id": "rel-sample"}
    wr = [{"op": o, "verified": True} for o in ("create_proposal", "put_draft", "freeze", "evaluate")]
    core = {"scout": {**ok, "core_run_id": "run-scout"}, "verifier": {**ok, "core_run_id": "run-verifier"}, "design": {**ok, "core_run_id": "run-design"},
            "attempts": [{"candidate_id": "cand-1", "proposal_id": "prop-core-1", "candidate_hash": "a" * 64, "write_receipts": wr,
                          "native": {"verdict": "pass", "eval_run_ref": "evr-1", "report_digest": "b" * 64}},
                         {"candidate_id": "cand-2", "proposal_id": "prop-core-2", "candidate_hash": "c" * 64, "write_receipts": wr,
                          "native": {"verdict": "pass", "eval_run_ref": "evr-2", "report_digest": "d" * 64}}]}
    exporter = {"audit_events": 42, "verification_receipts": [{"run_id": "run-scout", "check_result": {"ok": True, "broken_at": None, "reason": None},
                                                               "chain_head_hash": "e" * 64}]}
    res = bundle.assemble(namespace="golden", tenant="tenant-local", generated_at="2026-01-01T00:00:00Z", scout=sc, verify=ver,
                          alternatives=analysis.alternatives(conn, c2), attempts=attempts, core=core, exporter=exporter, dataset={"rows": 8000, "seed": 20260101})
    G.mkdir(exist_ok=True)
    (G / "results.json").write_text(json.dumps(res, indent=1, sort_keys=True), "utf-8")
    (G / "world.json").write_text(json.dumps(translate.build_world(res), indent=1, sort_keys=True), "utf-8")


if __name__ == "__main__":
    main()
