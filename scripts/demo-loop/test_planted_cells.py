import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import planted_cells as pc  # noqa: E402


class PlantedCells(unittest.TestCase):
    def test_deterministic_and_k_safe(self):
        a, b = pc.build(7), pc.build(7)
        self.assertEqual(a, b)
        self.assertNotEqual(a, pc.build(8))
        for r in a:
            n, d = r["numerator"], r["denominator"]
            self.assertTrue(d >= 10 and n >= 10 and d - n >= 10, r)
            self.assertEqual(set(r), {"metric", "dims", "half", "period", "numerator", "denominator"})
            self.assertEqual(r["metric"], "M4")

    def test_only_the_planted_category_is_high_in_both_halves(self):
        for half in ("discovery", "holdout"):
            tot = {}
            for r in pc.build(7):
                if r["half"] == half:
                    n, d = tot.get(r["dims"]["category"], (0, 0))
                    tot[r["dims"]["category"]] = (n + r["numerator"], d + r["denominator"])
            rates = {k: n / d for k, (n, d) in tot.items()}
            self.assertGreater(rates[pc.PLANTED], 0.55)
            self.assertTrue(all(v < 0.40 for k, v in rates.items() if k != pc.PLANTED), rates)

    def test_plant_two_raises_a_second_category_and_default_is_unchanged(self):
        self.assertEqual(pc.build(7), pc.build(7, 1))
        tot = {}
        for r in pc.build(7, 2):
            n, d = tot.get(r["dims"]["category"], (0, 0))
            tot[r["dims"]["category"]] = (n + r["numerator"], d + r["denominator"])
        high = sorted(k for k, (n, d) in tot.items() if n / d > 0.55)
        self.assertEqual(high, sorted([pc.PLANTED, pc.SECOND_PLANTED]))

    def test_uncovered_topic_profile_plants_one_m1_cell_no_agent_covers(self):
        # AGT1: the mapping row M1 x reason_category Tecnico yields `new_agent` first (the topic has no covering agent).
        rows = pc.build_uncovered(7)
        self.assertEqual(rows, pc.build_uncovered(7))
        self.assertTrue(all(r["metric"] == "M1" and set(r["dims"]) == {"reason_category", "channel"} for r in rows))
        for r in rows:
            n, d = r["numerator"], r["denominator"]
            self.assertTrue(d >= 10 and n >= 10 and d - n >= 10, r)
        by = {}
        for r in rows:
            by.setdefault((r["dims"]["reason_category"], r["dims"]["channel"], r["half"]), []).append(r["numerator"] / r["denominator"])
        for half in ("discovery", "holdout"):
            self.assertGreater(min(by[("Tecnico", "Phone", half)]), 0.40)
            others = [v for k, vs in by.items() if k[2] == half and k[:2] != ("Tecnico", "Phone") for v in vs]
            self.assertLess(max(others), 0.30)

    def test_cli_uncovered_profile(self):
        with tempfile.TemporaryDirectory() as t:
            out = Path(t) / "c.ndjson"
            self.assertEqual(pc.main(["--out", str(out), "--profile", "uncovered-topic"]), 0)
            self.assertEqual(json.loads(out.read_text(encoding="utf-8").splitlines()[0])["metric"], "M1")

    def test_cli_writes_ndjson(self):
        with tempfile.TemporaryDirectory() as t:
            out = Path(t) / "c.ndjson"
            self.assertEqual(pc.main(["--out", str(out)]), 0)
            rows = [json.loads(x) for x in out.read_text(encoding="utf-8").splitlines()]
            self.assertEqual(len(rows), 350)


if __name__ == "__main__":
    unittest.main()
