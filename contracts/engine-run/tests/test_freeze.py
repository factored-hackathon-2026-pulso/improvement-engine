"""C-2 and C-12 are frozen by digest at GT0. Any byte change to a covered file fails these tests."""
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import freeze  # noqa: E402

ROOT = HERE.parents[1]


class Freeze(unittest.TestCase):
    def pinned(self):
        return json.loads((HERE / "FREEZE.json").read_text(encoding="utf-8"))

    def test_both_contracts_are_pinned(self):
        p = self.pinned()["contracts"]
        self.assertEqual(sorted(p), ["C-12", "C-2"])
        for c in p.values():
            self.assertEqual(c["status"], "frozen")
            self.assertTrue(c["digest"].startswith("sha256:"))
            self.assertTrue(c["files"])

    def test_current_files_match_the_pin(self):
        self.assertEqual(freeze.verify(ROOT), [])

    def test_covered_files_are_the_contract_surface(self):
        files = self.pinned()["contracts"]
        self.assertIn("contracts/engine-run/engine_run.py", files["C-2"]["files"])
        self.assertIn("contracts/engine-run/report.schema.json", files["C-2"]["files"])
        for f in ("protocol", "scanner", "shim"):
            self.assertIn(f"roleplay-llm/roleplay_llm/{f}.py", files["C-12"]["files"])

    def _copy(self):
        d = Path(tempfile.mkdtemp())
        for rel in {f for c in self.pinned()["contracts"].values() for f in c["files"]}:
            (d / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / rel, d / rel)
        shutil.copy(HERE / "FREEZE.json", d / "contracts" / "engine-run" / "FREEZE.json")
        return d

    def test_drift_in_c2_is_detected(self):
        d = self._copy()
        with (d / "contracts/engine-run/engine_run.py").open("a", encoding="utf-8") as f:
            f.write("\n# drift\n")
        problems = freeze.verify(d)
        self.assertTrue(any("C-2" in p and "engine_run.py" in p for p in problems), problems)

    def test_drift_in_c12_is_detected(self):
        d = self._copy()
        with (d / "roleplay-llm/roleplay_llm/scanner.py").open("a", encoding="utf-8") as f:
            f.write("\n# drift\n")
        problems = freeze.verify(d)
        self.assertTrue(any("C-12" in p and "scanner.py" in p for p in problems), problems)

    def test_missing_file_is_detected_and_line_endings_are_not_drift(self):
        d = self._copy()
        p = d / "roleplay-llm/roleplay_llm/protocol.py"
        p.write_bytes(p.read_bytes().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n"))
        self.assertEqual(freeze.verify(d), [])
        (d / "roleplay-llm/roleplay_llm/shim.py").unlink()
        self.assertTrue(any("shim.py" in x and "missing" in x for x in freeze.verify(d)))


class ReportSchema(unittest.TestCase):
    def test_schema_requires_the_report_keys_the_checker_reads(self):
        s = json.loads((HERE / "report.schema.json").read_text(encoding="utf-8"))
        for k in ("contract_revision", "target", "sha", "host", "label", "quality_claims", "steps", "ports", "authors"):
            self.assertIn(k, s["required"])
        step = s["properties"]["steps"]["items"]
        for k in ("id", "status", "data_class", "target", "sha", "contract_revision", "host"):
            self.assertIn(k, step["required"])
        self.assertIn("null", s["properties"]["authors"]["properties"]["candidate_created_at"]["type"])


    def test_report_producer_is_covered_by_c2(self):
        pin = json.loads((HERE / "FREEZE.json").read_text(encoding="utf-8"))
        self.assertIn("e2e-core/src/claude_standin/thread01.py", pin["contracts"]["C-2"]["files"])

    def test_a_pin_that_drops_a_covered_file_is_rejected(self):
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            for rels in freeze.CONTRACTS.values():
                for rel in rels:
                    (t / rel).parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / rel, t / rel)
            pin = freeze.compute(t)
            c = pin["contracts"]["C-12"]
            c["files"].pop("roleplay-llm/roleplay_llm/scanner.py")
            c["digest"] = freeze.contract_digest(c["files"])
            (t / freeze.PIN).write_text(json.dumps(pin), encoding="utf-8")
            self.assertTrue(any("scanner.py" in x for x in freeze.verify(t)))


if __name__ == "__main__":
    unittest.main()
