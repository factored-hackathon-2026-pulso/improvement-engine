import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import owners_map as om  # noqa: E402

TEXT = (om.ROOT / "OWNERS.md").read_text(encoding="utf-8")


class OwnersTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rules = om.load_rules(TEXT)

    def test_every_tracked_file_has_exactly_one_owner(self):
        files = om.tracked_files()
        self.assertGreaterEqual(len(files), 1683)
        unowned = [f for f in files if om.owner(self.rules, f) is None]
        tied = [f for f in files if om.owner(self.rules, f) == "TIE"]
        self.assertEqual(unowned[:10], [])
        self.assertEqual(tied[:10], [])

    def test_planned_new_paths_resolve_to_intended_lane(self):
        for path, lane in om.newpaths(TEXT):
            self.assertEqual(om.owner(self.rules, path), lane, path)

    def test_tie_and_unowned_detection_bite(self):
        rules = [
            ("A", "x/*.md", om.conv("x/*.md"), 5),
            ("B", "x/a.*", om.conv("x/a.*"), 5),
        ]
        self.assertEqual(om.owner(rules, "x/a.md"), "TIE")
        self.assertIsNone(om.owner(rules, "y/z"))

    def test_most_specific_glob_wins(self):
        self.assertEqual(om.owner(self.rules, "contracts/engine-steps/x.json"), "L-GOV")
        self.assertEqual(om.owner(self.rules, "contracts/README.md"), "X-SRC")

    def test_new_path_rule_documented(self):
        self.assertIn("## Rules for new paths", TEXT)


if __name__ == "__main__":
    unittest.main()
