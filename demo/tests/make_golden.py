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
    # steps 8-10 sample: a SCRIPTED (simulated) human decision that reached staging, and the second observation batch
    obs = analysis.observe(dataset.build(seed=20260102, post=True), sc["hypotheses"], ver)
    decided = json.loads(json.dumps(res))
    decided["decision"] = {
        "mode": "scripted", "simulated_human": True, "actor": "local-supervisor", "stage": "staging_confirmed", "proposal_id": "prop-core-2",
        "candidate_hash": "c" * 64, "release_id": "rel-new", "staging_alias": "rel-new", "prod_alias": "rel-base", "promoted": False, "error": None,
        "reason": "scripted", "trail": [
            {"event": "decision_requested", "at": "2026-01-01T00:00:00Z", "operation": "approve", "candidate_hash": "c" * 64, "proposal_rev": 3,
             "intention_id": "int-approve", "command_ref": "cmd-approve", "staging_alias": "rel-base", "prod_alias": "rel-base"},
            {"event": "human_decided", "at": "2026-01-01T00:00:01Z", "decision": "approve", "simulated": True, "mode": "scripted"},
            {"event": "approved", "at": "2026-01-01T00:00:02Z", "operation": "approve", "candidate_hash": "c" * 64, "intention_id": "int-approve",
             "published": False, "staging_alias": "rel-base", "prod_alias": "rel-base"},
            {"event": "published", "at": "2026-01-01T00:00:03Z", "operation": "publish", "release_id": "rel-new", "intention_id": "int-publish",
             "staging_confirmed": False},
            {"event": "staging_confirmed", "at": "2026-01-01T00:00:04Z", "release_id": "rel-new", "staging_alias": "rel-new", "prod_alias": "rel-base",
             "prod_unchanged": True}]}
    decided["successor"] = {"batch": 2, "core": {"state": "terminal_ok", "core_run_id": "run-successor-scout"},
                            "exporter": {"batches_before": 3, "batches_after": 5, "verification_ok": True}, "memory_updates": obs["memory_updates"],
                            "new_hypotheses": obs["new_hypotheses"], "successor_target": obs["successor_target"],
                            "evidence": [{"id": q["id"], "digest": q["rows_digest"]} for q in obs["scout"]["queries"]]}
    G.mkdir(exist_ok=True)
    (G / "results_decided.json").write_text(json.dumps(decided, indent=1, sort_keys=True), "utf-8")
    (G / "world_decided.json").write_text(json.dumps(translate.build_world(decided), indent=1, sort_keys=True), "utf-8")
    (G / "results.json").write_text(json.dumps(res, indent=1, sort_keys=True), "utf-8")
    (G / "world.json").write_text(json.dumps(translate.build_world(res), indent=1, sort_keys=True), "utf-8")


if __name__ == "__main__":
    main()
