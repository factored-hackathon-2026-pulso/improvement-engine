import csv
import tempfile
import unittest
from pathlib import Path

from generate import _apply_global_tests, _coverage_count, _iter_unique_rows


class SourceIngestionTests(unittest.TestCase):
    def write_contact_parts(self, root: Path, rows: list[dict[str, str]]) -> None:
        directory = root / "call_center_interactions" / "year=2024"
        directory.mkdir(parents=True)
        columns = ["interaction_id", "customer_id", "reason_category", "channel", "was_resolved"]
        for index, row in enumerate(rows):
            with (directory / f"part-{index}.csv").open("w", newline="", encoding="utf-8") as stream:
                writer = csv.DictWriter(stream, fieldnames=columns)
                writer.writeheader()
                writer.writerow(row)

    def test_exact_duplicate_primary_key_collapses_without_echoing_source(self):
        row = {"interaction_id": "synthetic-1", "customer_id": "synthetic-c1", "reason_category": "queja", "channel": "phone", "was_resolved": "false"}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_contact_parts(root, [row, row])
            self.assertEqual(len(list(_iter_unique_rows(root, "call_center_interactions"))), 1)

    def test_conflicting_duplicate_primary_key_fails_closed(self):
        first = {"interaction_id": "synthetic-1", "customer_id": "synthetic-c1", "reason_category": "queja", "channel": "phone", "was_resolved": "false"}
        second = {**first, "was_resolved": "true"}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_contact_parts(root, [first, second])
            with self.assertRaisesRegex(ValueError, "conflicting duplicate key"):
                list(_iter_unique_rows(root, "call_center_interactions"))

    def test_discovery_bh_family_is_global_across_all_181_metric_cells(self):
        rows = []
        for index in range(181):
            rows.append({
                "metric_id": "E1" if index == 180 else "M1",
                "cell": {"scope": "overall"} if index == 180 else {"channel": f"cell-{index}"},
                "discovery_p_value": 0.001 if index == 0 else None,
                "multiple_testing": {"family_size": 1, "adjusted_q": None},
                "k_min_ok": True if index == 0 else False,
                "effect": {"difference": 0.10} if index == 0 else None,
                "replication": {"p_value": 0.001, "effect": {"difference": 0.10}, "adjusted_q": None},
                "status": "candidate", "replicated": False, "reason": "test",
            })
        _apply_global_tests(rows)
        self.assertEqual(rows[0]["multiple_testing"]["family_size"], 181)
        self.assertEqual(rows[0]["multiple_testing"]["adjusted_q"], 0.181)
        self.assertEqual(rows[0]["status"], "corroborated_descriptive")

    def test_coverage_counts_below_k_have_explicit_suppression_reason(self):
        self.assertEqual(_coverage_count(10, "eligible"), {"value": 10, "suppressed": False, "suppression_reason": None})
        self.assertEqual(_coverage_count(9, "eligible"), {"value": None, "suppressed": True, "suppression_reason": "eligible_below_k"})


if __name__ == "__main__":
    unittest.main()
