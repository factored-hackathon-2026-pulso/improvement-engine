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

    def test_cli_writes_ndjson(self):
        with tempfile.TemporaryDirectory() as t:
            out = Path(t) / "c.ndjson"
            self.assertEqual(pc.main(["--out", str(out)]), 0)
            rows = [json.loads(x) for x in out.read_text(encoding="utf-8").splitlines()]
            self.assertEqual(len(rows), 350)


if __name__ == "__main__":
    unittest.main()
