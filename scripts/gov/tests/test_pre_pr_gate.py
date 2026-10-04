"""Tests for G0p. Run: python -m unittest discover -s scripts/gov/tests"""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

GATE = Path(__file__).resolve().parents[1] / "pre-pr-gate.ps1"
OK = "exit 0"
BAD = "exit 3"


def run(packages="pkg-a,pkg-b", **legs):
    with tempfile.TemporaryDirectory() as d:
        rec = Path(d, "receipt.json")
        args = ["pwsh", "-NoProfile", "-File", str(GATE), "-ReceiptPath", str(rec),
                "-TouchedPackages", packages]
        for k, v in legs.items():
            if v is not None:
                args += [f"-{k}", v]
        r = subprocess.run(args, capture_output=True, text=True)
        data = json.loads(rec.read_text(encoding="utf-8")) if rec.exists() else None
        return r, data


class Gate(unittest.TestCase):
    def test_all_legs_pass(self):
        r, d = run(CiCommand=OK, PytestCommand=OK, RatchetCommand=OK)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(d["verdict"], "pass")
        self.assertEqual(d["packages_touched"], ["pkg-a", "pkg-b"])
        self.assertEqual(d["legs"]["pytest"]["status"], "pass")

    def test_missing_pytest_leg_exits_1(self):
        r, d = run(packages="", CiCommand=OK, RatchetCommand=OK)
        self.assertEqual(r.returncode, 1)
        self.assertEqual(d["legs"]["pytest"]["status"], "missing")
        self.assertEqual(d["verdict"], "fail")

    def test_missing_ratchet_leg_exits_1(self):
        r, d = run(CiCommand=OK, PytestCommand=OK)
        self.assertEqual(r.returncode, 1)
        self.assertEqual(d["legs"]["ratchet"]["status"], "missing")

    def test_package_injection_rejected(self):
        r, d = run(packages="x'; exit 0 #", CiCommand=OK, RatchetCommand=OK)
        self.assertEqual(r.returncode, 1)
        self.assertTrue(d is None or d["verdict"] == "fail")

    def test_receipt_records_head_sha_rust_scope_note_and_leg_commands(self):
        head = subprocess.run(["git", "-C", str(GATE.parents[2]), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        r, d = run(CiCommand=OK, PytestCommand=OK, RatchetCommand=OK, BaseRef="HEAD", Note="no cargo on this machine")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(d["head_sha"], head)
        self.assertEqual(d["base_ref"], "HEAD")
        self.assertEqual(d["rust_files_changed"], [])
        self.assertEqual(d["notes"], "no cargo on this machine")
        self.assertEqual(d["legs"]["ci"]["command"], OK)

    def test_rust_files_changed_listed_against_a_real_base(self):
        r, d = run(CiCommand=OK, PytestCommand=OK, RatchetCommand=OK, BaseRef="HEAD~1")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIsInstance(d["rust_files_changed"], list)

    def test_unresolvable_base_ref_fails(self):
        r, d = run(CiCommand=OK, PytestCommand=OK, RatchetCommand=OK, BaseRef="no-such-ref-xyz")
        self.assertEqual(r.returncode, 1)

    def test_failing_leg_exits_1(self):
        r, d = run(CiCommand=BAD, PytestCommand=OK, RatchetCommand=OK)
        self.assertEqual(r.returncode, 1)
        self.assertEqual(d["legs"]["ci"]["status"], "fail")


if __name__ == "__main__":
    unittest.main()
