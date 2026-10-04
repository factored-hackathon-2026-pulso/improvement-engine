"""GT05 gate (DEMO-1a): every item is recomputed from a Rust-steps run report. Built on the real GT0 default report so the
G1 check() runs on a realistic shape; Rust-run mutations are applied on top."""
import copy
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
import gt05_gate as gate  # noqa: E402

DEFAULT = json.loads((ROOT / "docs" / "reports" / "gates" / "gt0" / "report.json").read_text(encoding="utf-8"))
EXE_BYTES = b"fake steps_cli binary"
EXE_SHA = hashlib.sha256(EXE_BYTES).hexdigest()
FLIPS = ("signals", "recompute", "validation", "compile", "gate")


def rust_report(sha=EXE_SHA):
    r = copy.deepcopy(DEFAULT)
    base = {"host": "python", "target": "local", "sha": r["sha"], "contract_revision": r["contract_revision"]}
    rust = {"status": "real-narrow", "semantics": "claude-standin", "steps_label": "steps/v1", "steps_exe_sha256": sha,
            "receipt": {"provider": "claude-standin"}}
    out = []
    for s in r["steps"]:
        s = dict(s)
        if s["id"] in ("signals", "compile", "gate"):
            keep = s.get("receipt")
            s.update(rust)
            if s["id"] == "signals":
                s["receipt"] = keep
        out.append(s)
        if s["id"] == "verifier":
            out.append({**base, **rust, "id": "recompute", "n": 3, "data_class": "generated_sample"})
        if s["id"] == "opportunity":
            out.append({**base, **rust, "id": "validation", "n": 4, "data_class": "generated_sample"})
    r["steps"] = out
    return r


def build(d, mutate=None):
    d = Path(d)
    exe = d / "steps_cli.exe"
    exe.write_bytes(EXE_BYTES)
    files = {"rust-report.json": rust_report(), "default-report.json": copy.deepcopy(DEFAULT)}
    for n, doc in files.items():
        (d / n).write_text(json.dumps(doc, indent=1, sort_keys=True), encoding="utf-8")
    rep = "sha256:" + hashlib.sha256((d / "rust-report.json").read_bytes()).hexdigest()
    (d / "rust-run.json").write_text(json.dumps({
        "schema": "gt05-run/v1", "mode": "rust", "steps_exe": str(exe), "steps_exe_sha256": EXE_SHA, "report_sha256": rep,
        "command": "python gt05_collect.py", "exit_code": 0, "calls": 5, "misses": 0}), encoding="utf-8")
    (d / "ratchet.json").write_text(json.dumps({"schema": "ratchet-receipt/v1", "command": "pytest ratchet", "exit_code": 0,
                                                "passed": 7, "failed": 0}), encoding="utf-8")
    orig = rust_report()
    step(orig, "signals")["data_class"] = "original"
    (d / "original-report.json").write_text(json.dumps(orig), encoding="utf-8")
    (d / "original-run.json").write_text(json.dumps({"schema": "gt05-original-run/v1", "steps_mode": "rust",
                                                     "report": "original-report.json"}), encoding="utf-8")
    rv = d / "reviews"
    rv.mkdir(exist_ok=True)
    (rv / "x.review.json").write_text(json.dumps({
        "schema": "review-log/v1", "wps": list(gate.REQUIRED_CRV1), "author": {"id": "a"}, "reviewer": {"id": "b"},
        "provenance": "contemporaneous", "findings": [], "verdict": "closed"}), encoding="utf-8")
    m = {"schema": "gt05-manifest/v1", "reviews_dir": "reviews", "gt0_manifest": "gt0-manifest.json",
         "artifacts": {"rust_report": "rust-report.json", "default_report": "default-report.json", "run": "rust-run.json",
                       "ratchet_receipt": "ratchet.json", "original_run": "original-run.json"}}
    if mutate:
        mutate(d, m)
    (d / "manifest.json").write_text(json.dumps(m), encoding="utf-8")
    return d / "manifest.json"


def GT0_OK(manifest, repo):
    return [{"item": "g0p", "ok": True, "detail": "x"}, {"item": "capacity", "ok": False, "detail": "missing"}]


def failed(results):
    return {r["item"] for r in results if not r["ok"]}


def step(doc, sid):
    return next(s for s in doc["steps"] if s["id"] == sid)


class Gate(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.d = Path(self._t.name)

    def tearDown(self):
        self._t.cleanup()

    def run_gate(self, mutate=None, gt0=GT0_OK, **kw):
        return gate.evaluate(build(self.d, mutate), repo=ROOT, gt0_evaluator=gt0, **kw)

    def edit(self, fn, name="rust-report.json"):
        def mut(d, m):
            doc = json.loads((d / name).read_text())
            fn(doc)
            (d / name).write_text(json.dumps(doc, indent=1, sort_keys=True))
            if name == "rust-report.json":
                run = json.loads((d / "rust-run.json").read_text())
                run["report_sha256"] = "sha256:" + hashlib.sha256((d / name).read_bytes()).hexdigest()
                (d / "rust-run.json").write_text(json.dumps(run))
        return mut

    def test_complete_good_set_passes_every_item(self):
        res = self.run_gate()
        self.assertEqual(failed(res), set(), res)

    def test_still_standin_step_fails_the_flip_check(self):
        for sid in FLIPS:
            res = self.run_gate(self.edit(lambda r, sid=sid: step(r, sid).update(status="stand-in")))
            self.assertIn("rust_flips", failed(res), sid)

    def test_wrong_semantics_or_host_fails(self):
        for k, v in (("semantics", "codex"), ("host", "rust")):
            res = self.run_gate(self.edit(lambda r, k=k, v=v: step(r, "gate").update({k: v})))
            self.assertIn("rust_flips", failed(res), k)

    def test_missing_semantics_key_fails(self):
        self.assertIn("rust_flips", failed(self.run_gate(self.edit(lambda r: step(r, "compile").pop("semantics")))))

    def test_default_python_steps_are_not_a_rust_run(self):
        res = self.run_gate(self.edit(lambda r: r.update(steps=copy.deepcopy(DEFAULT["steps"]))))
        self.assertIn("rust_flips", failed(res))

    def test_exe_sha_must_match_the_binary_on_disk(self):
        res = self.run_gate(self.edit(lambda r: step(r, "recompute").update(steps_exe_sha256="0" * 64)))
        self.assertIn("exe_sha256", failed(res))

    def test_stale_binary_on_disk_fails(self):
        self.assertIn("exe_sha256", failed(self.run_gate(lambda d, m: (d / "steps_cli.exe").write_bytes(b"rebuilt later"))))

    def test_missing_binary_fails(self):
        self.assertIn("exe_sha256", failed(self.run_gate(lambda d, m: (d / "steps_cli.exe").unlink())))

    def test_explicit_exe_overrides_the_record(self):
        res = self.run_gate(lambda d, m: (d / "other.exe").write_bytes(b"x"), steps_exe=str(self.d / "other.exe"))
        self.assertIn("exe_sha256", failed(res))

    def test_undisclosed_python_fallback_fails(self):
        def silent(r):
            s = step(r, "compile")
            s.update(status="stand-in")
            s.pop("steps_exe_sha256")
        self.assertIn("fallback_disclosure", failed(self.run_gate(self.edit(silent))))

    def test_disclosed_fallback_passes_disclosure_but_still_fails_the_flip(self):
        def disclosed(r):
            s = step(r, "compile")
            s.update(status="stand-in", steps_fallback="python(core dry-run hook)")
            s.pop("steps_exe_sha256")
        res = self.run_gate(self.edit(disclosed))
        self.assertNotIn("fallback_disclosure", failed(res))
        self.assertIn("rust_flips", failed(res))

    def test_fallback_flag_on_a_step_claiming_rust_is_contradictory(self):
        res = self.run_gate(self.edit(lambda r: step(r, "gate").update(steps_fallback="python(x)")))
        self.assertIn("fallback_disclosure", failed(res))

    def test_g1_check_violations_fail(self):
        self.assertIn("g1_check", failed(self.run_gate(self.edit(lambda r: r.update(label="DEMO-2")))))

    def test_red_step_in_the_rust_run_fails(self):
        self.assertIn("no_red_steps", failed(self.run_gate(self.edit(lambda r: step(r, "approval").update(status="red")))))

    def test_run_record_must_bind_the_report_bytes(self):
        def mut(d, m):
            p = d / "rust-report.json"
            p.write_text(p.read_text() + " ")
        self.assertIn("run_record", failed(self.run_gate(mut)))

    def test_run_record_needs_rust_mode_zero_exit_and_no_misses(self):
        for k, v in (("mode", "python"), ("exit_code", 1), ("misses", 2)):
            def mut(d, m, k=k, v=v):
                run = json.loads((d / "rust-run.json").read_text())
                run[k] = v
                (d / "rust-run.json").write_text(json.dumps(run))
            self.assertIn("run_record", failed(self.run_gate(mut)), k)

    def test_gt0_dependency_is_all_items_but_capacity(self):
        res = self.run_gate(gt0=lambda m, r: [{"item": "g0p", "ok": False, "detail": "stale"}, {"item": "capacity", "ok": False, "detail": "x"}])
        self.assertIn("gt0_dependency", failed(res))
        self.assertIn("gt0_dependency", failed(self.run_gate(gt0=lambda m, r: [])))  # an empty evaluation is not a pass
        self.assertNotIn("gt0_dependency", failed(self.run_gate(gt0=lambda m, r: [{"item": "g0p", "ok": True, "detail": "x"}])))

    def test_ratchet_default_run_must_stay_python(self):
        for fn in (lambda r: step(r, "compile").update(status="real-narrow"),
                   lambda r: step(r, "signals").update(semantics="claude-standin"),
                   lambda r: step(r, "gate").update(steps_exe_sha256=EXE_SHA),
                   lambda r: r.update(label="DEMO-1a")):
            self.assertIn("ratchet_default", failed(self.run_gate(self.edit(fn, "default-report.json"))))

    def test_unflipped_steps_must_agree_between_the_runs(self):
        self.assertIn("ratchet_default", failed(self.run_gate(self.edit(lambda r: step(r, "scout").update(status="stand-in")))))

    def test_crv1_missing_wp_or_open_finding_fails(self):
        def mut(d, m):
            p = d / "reviews" / "x.review.json"
            log = json.loads(p.read_text())
            log["wps"] = log["wps"][:-1]
            p.write_text(json.dumps(log))
        self.assertIn("crv1", failed(self.run_gate(mut)))

    def test_rg1_every_real_narrow_step_must_be_served_by_rust(self):
        self.assertIn("rg1", failed(self.run_gate(self.edit(lambda r: step(r, "approval").update(status="real-narrow")))))

    def test_rg1_open_parts_are_missing_not_assumed(self):
        def mut(d, m):
            m["artifacts"].pop("ratchet_receipt")
            m["artifacts"].pop("original_run")
        by = {r["item"]: r for r in self.run_gate(mut)}
        for k in ("rg1_ratchet_tests", "rg1_original"):
            self.assertFalse(by[k]["ok"])
            self.assertTrue(by[k]["detail"].startswith("MISSING"), by[k])
        self.assertTrue(by["rg1"]["ok"], by["rg1"])

    def test_original_run_needs_an_original_data_class_and_rust_served_steps(self):
        def mut(d, m):
            (d / "original-report.json").write_text((d / "rust-report.json").read_text())
        self.assertIn("rg1_original", failed(self.run_gate(mut)))

    def test_ratchet_receipt_must_be_a_passing_run(self):
        def mut(d, m):
            p = d / "ratchet.json"
            doc = json.loads(p.read_text())
            doc["exit_code"] = 1
            p.write_text(json.dumps(doc))
        self.assertIn("rg1_ratchet_tests", failed(self.run_gate(mut)))

    def test_empty_manifest_fails_every_item(self):
        p = self.d / "m.json"
        p.write_text("{}")
        res = gate.evaluate(p, repo=ROOT, gt0_evaluator=GT0_OK)
        self.assertTrue(res and all(not r["ok"] for r in res), res)

    def test_cli_exit_code_and_output(self):
        m = build(self.d)
        out = self.d / "result.json"
        r = subprocess.run([sys.executable, str(GOV / "gt05_gate.py"), "--manifest", str(m), "--skip-gt0", "--out", str(out)],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)  # a skipped GT0 dependency can never pass
        self.assertIn("FAIL gt0_dependency", r.stdout)
        self.assertEqual(json.loads(out.read_text())["schema"], "gt05-gate-result/v1")


if __name__ == "__main__":
    unittest.main()
