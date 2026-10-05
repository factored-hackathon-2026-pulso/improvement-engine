import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import post_cells as pc  # noqa: E402


def row(cat, period, num, den=200):
    return {"metric": "M4", "dims": {"category": cat}, "half": "discovery", "period": period, "numerator": num, "denominator": den}


class CutTests(unittest.TestCase):
    def test_only_treated_after_release_is_cut(self):
        rows = [row("A", "2025-05", 120), row("A", "2025-07", 120), row("B", "2025-07", 60)]
        out = pc.cut_rows(rows, "2025-06", "A", 8)
        self.assertEqual([r["numerator"] for r in out], [120, 104, 60])

    def test_floor(self):
        out = pc.cut_rows([row("A", "2025-07", 12)], "2025-06", "A", 50)
        self.assertEqual(out[0]["numerator"], 10)


if __name__ == "__main__":
    unittest.main()
