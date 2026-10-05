import unittest

from scripts.aggregate.agent_runs.aggregation import aggregate_export
from scripts.aggregate.agent_runs.cell_table import parse_cell_ndjson, render_cell_ndjson
from scripts.aggregate.bank_cells import to_ndjson as bank_cells_to_ndjson

from test_aggregation import _balanced_outcomes, _make_export


class SharedCellTableContractTests(unittest.TestCase):
    def test_bank_and_agent_rows_load_through_one_six_field_reader(self):
        bank_rows = [{
            "metric": "M1",
            "dims": {"reason_category": "Queja", "channel": "Phone"},
            "half": "discovery",
            "period": "2025-01",
            "numerator": 12,
            "denominator": 30,
        }]
        agent_report = aggregate_export(_make_export(_balanced_outcomes(per_group=10)))
        rows = parse_cell_ndjson(
            bank_cells_to_ndjson(bank_rows) + render_cell_ndjson(agent_report["cells"])
        )
        self.assertEqual(len(rows), len(bank_rows) + len(agent_report["cells"]))
        self.assertEqual(set(rows[0]), {"metric", "dims", "half", "period", "numerator", "denominator"})

    def test_render_is_canonical_and_stable(self):
        cells = [
            {"metric": "resolved", "dims": {}, "half": "holdout", "period": "2026-01", "numerator": 10, "denominator": 50},
            {"metric": "failed", "dims": {}, "half": "discovery", "period": "2026-01", "numerator": 11, "denominator": 51},
        ]
        first = render_cell_ndjson(cells)
        self.assertEqual(first, render_cell_ndjson(cells))
        self.assertEqual(first, render_cell_ndjson(list(reversed(cells))))
        self.assertEqual(parse_cell_ndjson(first), sorted(cells, key=lambda row: (row["period"], row["half"], row["metric"])))

    def test_shared_reader_rejects_extra_fields_and_small_counts(self):
        valid = '{"metric":"M1","dims":{"channel":"Phone"},"half":"discovery","period":"2025-01","numerator":12,"denominator":20}'
        with self.assertRaises(ValueError):
            parse_cell_ndjson(valid.replace('"metric":"M1"', '"metric":"M1","run_id":"private"'))
        with self.assertRaises(ValueError):
            parse_cell_ndjson(valid.replace('"numerator":12', '"numerator":4'))


if __name__ == "__main__":
    unittest.main()
