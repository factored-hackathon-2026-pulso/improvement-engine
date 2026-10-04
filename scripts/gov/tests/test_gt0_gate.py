"""GT0 gate evidence script: fails on any missing receipt; every item is recomputed from artifacts."""
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

GOV = Path(__file__).resolve().parents[1]
ROOT = GOV.parents[1]
sys.path.insert(0, str(GOV))
import gt0_collect as collect  # noqa: E402
import gt0_gate as gate  # noqa: E402

SHA = "a" * 40
HEAD = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()


def step(i, status, dc="generated_sample", **kw):
    return {"id": i, "status": status, "data_class": dc, "target": "local", "sha": SHA, "contract_revision": "c2-1",
            "host": "python", **kw}


def report():
    r = {"contract_revision": "c2-1", "target": "local", "sha": SHA, "host": "python", "label": "DEMO-0",
         "quality_claims": "forbidden", "gate": {"verdict": "fail"},
         "overrides": [{"step": "approval", "of": "gate", "verdict": "fail", "by": "human", "label": "human_override",
                        "reason": "demo publication under a labelled override", "simulated": True}],
         "steps": [step("scout", "agent_roleplay", receipt={"provider": "agent_roleplay", "scanner_id": "tps-1"},
                        actor="scout-1", model="m-a", stage_output={"source": "model"}),
                   step("verifier", "agent_roleplay", receipt={"provider": "agent_roleplay", "scanner_id": "tps-1"},
                        actor="verifier-1", model="m-b", stage_output={"source": "model"}),
                   step("compile", "stand-in", receipt={"provider": "claude-standin"}),
                   step("approval", "simulated", receipt={"provider": "claude-standin"}),
                   step("publish", "stand-in", receipt={"provider": "claude-standin"})],
         "ports": [{"port": "registry", "provenance": "in-process-double", "price_source": "n/a"}],
         "authors": {"world": "a", "suite": "b", "effect": "c", "judge": "d",
                     "suite_sealed_at": "2026-01-01T00:00:00Z", "candidate_created_at": "2026-01-02T00:00:00Z"}}
    r["doubles"] = [{"part": p, "status": "x"} for p in ("model", "jev", "issuer", "product", "host", "gate", "data.origin")]
    return r


def rebind(d):
    """Re-point the replay record at the current bytes of report.json."""
    rp = json.loads((d / "replay.json").read_text())
    rp["report_sha256"] = "sha256:" + hashlib.sha256((d / "report.json").read_bytes()).hexdigest()
    (d / "replay.json").write_text(json.dumps(rp))


def build(d, mutate=None):
    """Write a complete, good artifact set under d and return the manifest path."""
    d = Path(d)
    files = {
        "g0p.json": {"schema": "pre-pr-gate/v1", "verdict": "pass", "head_sha": HEAD, "base_ref": HEAD, "rust_files_changed": [],
                     "legs": {k: {"status": "pass", "exit_code": 0, "command": "c"} for k in ("ci", "pytest", "ratchet")}},
        "report.json": report(),
        "live.json": {"schema": "live-window-log/v1", "windows": [
            {"id": "w1", "kind": "roleplay-synthetic", "calls": 6, "minutes": 4.1, "scanner_rejections": 0, "responders": 2}]},
        "capacity.json": {"schema": "capacity/v1", "sessions": [{"n": i, "lane_hours": 5.0} for i in range(1, 13)],
                          "baseline_lane_hours_per_session": 5.0},
    }
    for n, doc in files.items():
        (d / n).write_text(json.dumps(doc), encoding="utf-8")
    rep_sha = "sha256:" + hashlib.sha256((d / "report.json").read_bytes()).hexdigest()
    (d / "replay.json").write_text(json.dumps({"schema": "gt0-replay/v1", "run_id": "run-1", "mode": "replay", "calls": 12,
                                               "misses": 0, "fixtures_digest": collect.fixtures_digest(collect.FIXTURES), "report_sha256": rep_sha,
                                               "mapping_mutation_violations": []}), encoding="utf-8")
    (d / "w0-pr-1.json").write_text((d / "g0p.json").read_text(), encoding="utf-8")
    (d / "trn0.json").write_text(json.dumps({"schema": "train-receipt/v1", "verdict": "pass", "prs": [
        {"n": 1, "lanes": ["a"], "lane_hours": 10, "w0_receipt": "w0-pr-1.json"}]}), encoding="utf-8")
    rv = d / "reviews"
    rv.mkdir()
    (rv / "x.review.json").write_text(json.dumps({
        "schema": "review-log/v1", "wps": list(gate.crv0_closure.DEFAULT_REQUIRED), "author": {"id": "a"}, "reviewer": {"id": "b"}, "provenance": "contemporaneous",
        "findings": [], "verdict": "closed"}), encoding="utf-8")
    m = {"schema": "gt0-manifest/v1", "reviews_dir": "reviews", "required_wps": ["TPS"],
         "artifacts": {"g0p": "g0p.json", "trn0": "trn0.json", "report": "report.json", "replay": "replay.json",
                       "live_log": "live.json", "capacity": "capacity.json"}}
    if mutate:
        mutate(d, m)
    (d / "manifest.json").write_text(json.dumps(m), encoding="utf-8")
    return d / "manifest.json"


def failed(results):
    return {r["item"] for r in results if not r["ok"]}


class Gate(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.d = Path(self._t.name)

    def tearDown(self):
        self._t.cleanup()

    def run_gate(self, mutate=None):
        return gate.evaluate(build(self.d, mutate), repo=ROOT)

    def edit(self, name, fn):
        def mut(d, m):
            doc = json.loads((d / name).read_text())
            fn(doc)
            (d / name).write_text(json.dumps(doc))
        return mut

    def test_first_red_missing_receipt_fails(self):
        self.assertIn("g0p", failed(self.run_gate(lambda d, m: (d / "g0p.json").unlink())))

    def test_empty_manifest_fails_every_item(self):
        res = self.run_gate(lambda d, m: m.update(artifacts={}))
        self.assertTrue({"g0p", "trn0", "honesty", "replay", "live_window", "capacity"} <= failed(res))

    def test_complete_set_passes(self):
        res = self.run_gate()
        self.assertEqual(failed(res), set(), [r for r in res if not r["ok"]])

    def test_cli_exit_codes(self):
        m = build(self.d)
        cmd = [sys.executable, str(GOV / "gt0_gate.py"), "--manifest", str(m)]
        r = subprocess.run(cmd, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        (self.d / "trn0.json").unlink()
        r = subprocess.run(cmd, capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)
        self.assertIn("trn0", r.stdout)

    def test_g0p_failed_leg_fails(self):
        self.assertIn("g0p", failed(self.run_gate(self.edit("g0p.json", lambda g: g["legs"]["ratchet"].update(status="missing")))))

    def test_trn0_requires_a_w0_receipt_for_every_pr(self):
        self.assertIn("trn0", failed(self.run_gate(lambda d, m: (d / "w0-pr-1.json").unlink())))

    def test_crv0_open_review_fails_and_missing_required_wp_fails(self):
        def open_f(doc):
            doc["findings"] = [{"id": "F", "loop": 1, "summary": "s", "status": "open"}]
            doc["verdict"] = "open"
        self.assertIn("crv0", failed(self.run_gate(lambda d, m: self.edit("reviews/x.review.json", open_f)(d, m))))
        self.setUp()
        self.assertIn("crv0", failed(self.run_gate(lambda d, m: m.update(required_wps=["TPS", "ZZ9"]))))

    def test_empty_required_wps_cannot_waive_the_default_closure(self):
        def mut(d, m):
            (d / "reviews" / "x.review.json").write_text(json.dumps({
                "schema": "review-log/v1", "wps": ["TPS"], "author": {"id": "a"}, "reviewer": {"id": "b"},
                "provenance": "contemporaneous", "findings": [], "verdict": "closed"}))
            m["required_wps"] = []
        self.assertIn("crv0", failed(self.run_gate(mut)))

    def test_g0p_leg_needs_a_command_and_exit_code_zero(self):
        self.assertIn("g0p", failed(self.run_gate(self.edit("g0p.json", lambda g: g["legs"]["ci"].pop("command")))))
        self.setUp()
        self.assertIn("g0p", failed(self.run_gate(self.edit("g0p.json", lambda g: g["legs"]["ci"].update(exit_code=1)))))

    def test_g0p_ancestry_is_checked_even_if_the_manifest_does_not_ask(self):
        res = self.run_gate(self.edit("g0p.json", lambda g: g.update(head_sha=SHA)))
        self.assertIn("g0p", failed(res))

    def test_g0p_rust_claim_is_recomputed_from_git(self):
        repo = self.d / "repo"
        repo.mkdir()
        g = lambda *a: subprocess.run(["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@x.invalid", *a],
                                      capture_output=True, text=True, check=True).stdout.strip()
        g("init", "-q")
        (repo / "a.txt").write_text("1")
        g("add", "-A"); g("commit", "-qm", "base")
        base = g("rev-parse", "HEAD")
        (repo / "x.rs").write_text("fn main(){}")
        g("add", "-A"); g("commit", "-qm", "rust")
        head = g("rev-parse", "HEAD")
        doc = json.loads(json.dumps({"schema": "pre-pr-gate/v1", "verdict": "pass", "head_sha": head, "base_ref": base,
                                     "rust_files_changed": [],
                                     "legs": {k: {"status": "pass", "exit_code": 0, "command": "c"} for k in gate.LEGS}}))
        p = self.d / "r.json"
        p.write_text(json.dumps(doc))
        self.assertFalse(gate.check_g0p(p, repo, True)["ok"])            # claims no Rust, git shows x.rs
        doc["rust_files_changed"] = ["x.rs"]
        p.write_text(json.dumps(doc))
        self.assertFalse(gate.check_g0p(p, repo, True)["ok"])            # Rust changed but no cargo leg ran
        doc["legs"]["ci"]["command"] = "cargo test"
        p.write_text(json.dumps(doc))
        self.assertTrue(gate.check_g0p(p, repo, True)["ok"])

    def test_replay_fixtures_digest_is_recomputed(self):
        res = self.run_gate(self.edit("replay.json", lambda r: r.update(fixtures_digest="sha256:" + "0" * 64)))
        self.assertIn("replay", failed(res))

    def test_honesty_runs_the_eight_tests_against_the_real_report(self):
        def bad(d, m):
            self.edit("report.json", lambda r: r["steps"][0].update(status="real"))(d, m)   # test 1: real with a roleplay provider
            rebind(d)
        res = self.run_gate(bad)
        self.assertIn("honesty", failed(res))
        self.assertIn("test_1", next(r for r in res if r["item"] == "honesty")["detail"])

    def test_replay_misses_or_unbound_report_fail(self):
        self.assertIn("replay", failed(self.run_gate(self.edit("replay.json", lambda r: r.update(misses=1)))))
        self.setUp()
        self.assertIn("replay", failed(self.run_gate(self.edit("replay.json", lambda r: r.update(report_sha256="sha256:" + "1" * 64)))))

    def test_mapping_mutation_violations_fail_honesty_test_3(self):
        res = self.run_gate(self.edit("replay.json", lambda r: r.update(mapping_mutation_violations=[{"rule": "H3"}])))
        self.assertIn("honesty", failed(res))
        self.assertIn("test_3", next(r for r in res if r["item"] == "honesty")["detail"])

    def test_scanner_ids_must_be_known(self):
        def mut(d, m):
            self.edit("report.json", lambda r: r["steps"][0]["receipt"].update(scanner_id="made-up"))(d, m)
            rebind(d)
        self.assertIn("scanner_ids", failed(self.run_gate(mut)))

    def test_doubles_must_list_the_plan_parts(self):
        def mut(d, m):
            self.edit("report.json", lambda r: r.update(doubles=[x for x in r["doubles"] if x["part"] != "jev"]))(d, m)
            rebind(d)
        self.assertIn("doubles", failed(self.run_gate(mut)))

    def test_demo0_scope_label_host_and_quality_claims(self):
        def mut(d, m):
            self.edit("report.json", lambda r: r.update(label="DEMO-1a"))(d, m)
            rebind(d)
        self.assertIn("doubles", failed(self.run_gate(mut)))

    def test_live_log_needs_a_roleplay_window(self):
        self.assertIn("live_window", failed(self.run_gate(self.edit("live.json", lambda r: r.update(windows=[])))))

    def test_capacity_needs_twelve_sessions(self):
        self.assertIn("capacity", failed(self.run_gate(self.edit("capacity.json", lambda c: c.update(sessions=c["sessions"][:5])))))

    def test_frozen_contracts_are_checked(self):
        self.assertTrue(gate.check_freeze(ROOT)["ok"])
        self.assertFalse(gate.check_freeze(Path(self._t.name))["ok"])


if __name__ == "__main__":
    unittest.main()
