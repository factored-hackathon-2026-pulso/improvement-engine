"""G0gr: audit script must fail when counts are absent; report must carry measured counts."""
import json
import pathlib
import tempfile
import unittest

import pub_surface_audit as audit

HERE = pathlib.Path(__file__).parent
KEYS = ("pub_types", "serde_derive_types", "compile_fail_refs", "files")


class Measure(unittest.TestCase):
    def test_counts_fixture(self):
        with tempfile.TemporaryDirectory() as d:
            p = pathlib.Path(d, "crates", "x", "src")
            p.mkdir(parents=True)
            (p / "a.rs").write_text(
                "#[derive(Debug, Serialize, Deserialize)]\npub struct A;\n"
                "pub enum B {}\npub(crate) struct C;\nstruct D;\n"
                "pub trait T {}\npub type U = u8;\n"
                "/// ```compile_fail\n/// x\n/// ```\n",
                encoding="utf-8",
            )
            m = audit.measure(pathlib.Path(d))
            self.assertEqual(m["pub_types"], 4)  # struct, enum, trait, type
            self.assertEqual(m["serde_derive_types"], 1)
            self.assertEqual(m["compile_fail_refs"], 1)
            self.assertEqual(m["files"], 1)

    def test_check_fails_when_counts_absent(self):
        self.assertNotEqual(audit.check({}), [])
        self.assertNotEqual(audit.check({"pub_types": 1}), [])

    def test_check_passes_when_complete(self):
        self.assertEqual(audit.check({k: 1 for k in KEYS}), [])


class Report(unittest.TestCase):
    def test_committed_report_has_counts(self):
        r = json.loads((HERE / "pub-surface-audit.json").read_text(encoding="utf-8"))
        self.assertEqual(audit.check(r["totals"]), [])
        self.assertIn("per_crate", r)


if __name__ == "__main__":
    unittest.main()
