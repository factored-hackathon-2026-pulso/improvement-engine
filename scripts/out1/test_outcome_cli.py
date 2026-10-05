"""OUT1: the JSON CLI contract between the engine's outcome step and an outcome estimator (offline, stdlib only)."""
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import outcome_cli_adapter as adapter  # noqa: E402

ROWS = [
    {"metric": "M1", "dims": {"reason_category": "Queja", "channel": "Phone"}, "half": "discovery", "period": "2024-01", "numerator": 5, "denominator": 10},
    {"metric": "M1", "dims": {"reason_category": "Tecnico", "channel": "Phone"}, "half": "discovery", "period": "2024-01", "numerator": 5, "denominator": 10},
]


def write(path, text):
    Path(path).write_text(text, encoding="utf-8")


class AdapterTests(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp(prefix="out1-")
        self.cells = os.path.join(self.d, "cells.ndjson")
        write(self.cells, "\n".join(json.dumps(r) for r in ROWS) + "\n")
        self.treated = os.path.join(self.d, "treated.json")
        write(self.treated, json.dumps({"metric": "M1", "dims": {"reason_category": "Queja", "channel": "Phone"}}))

    def test_selects_only_the_treated_cell_and_passes_release_and_window(self):
        seen = {}

        def fake(rows, release_period, window_months=3):
            seen.update(rows=rows, release=release_period, window=window_months)
            return {"release_period": release_period, "window_months": window_months, "multiplicity": {"family_size": 4}, "cells": [
                {"metric": "M1", "dims": {"reason_category": "Queja", "channel": "Phone"}, "status": "inconclusive", "reason": "underpowered_minimum_support", "n_pre": 0, "n_post": 0},
                {"metric": "M1", "dims": {"reason_category": "Tecnico", "channel": "Phone"}, "status": "inconclusive", "reason": "x", "n_pre": 0, "n_post": 0}]}

        out = adapter.run(self.cells, self.treated, "2025-01", 4, estimate=fake, normalize=dict)
        self.assertEqual(out["contract"], "pulso.outcome.v1")
        self.assertEqual([c["dims"]["reason_category"] for c in out["cells"]], ["Queja"])
        self.assertEqual((seen["release"], seen["window"], len(seen["rows"])), ("2025-01", 4, 2))

    def test_the_treated_cell_is_found_under_the_estimators_folded_labels_and_answered_under_the_callers(self):
        def fake(rows, release_period, window_months=3):
            return {"cells": [{"metric": "M1", "dims": {"reason_category": "complaint", "channel": "phone"}, "status": "inconclusive", "reason": "x"}]}

        fold = lambda d: {"reason_category": "complaint", "channel": "phone"}  # noqa: E731
        out = adapter.run(self.cells, self.treated, "2025-01", 3, estimate=fake, normalize=fold)
        self.assertEqual(out["cells"][0]["dims"], {"reason_category": "Queja", "channel": "Phone"})
        self.assertEqual(out["cells"][0]["estimator_dims"]["reason_category"], "complaint")

    def test_refuses_rows_that_are_not_cell_aggregates(self):
        write(self.cells, json.dumps({"customer_id": "c1", "metric": "M1"}) + "\n")
        with self.assertRaises(ValueError):
            adapter.run(self.cells, self.treated, "2025-01", 3, estimate=lambda *a, **k: {"cells": []}, normalize=dict)

    def test_cli_exit_codes_and_json_on_stdout(self):
        r = subprocess.run([sys.executable, str(HERE / "outcome_cli_adapter.py"), "--cells", self.cells, "--treated", self.treated, "--release-date", "2025-13"],
                           capture_output=True, text=True, cwd=self.d)
        self.assertEqual(r.returncode, 2)
        self.assertEqual(r.stdout.strip(), "")


class DoubleTests(unittest.TestCase):
    def test_double_prints_the_canned_verdict_and_logs_its_arguments(self):
        d = tempfile.mkdtemp(prefix="out1-")
        canned = os.path.join(d, "canned.json")
        log = os.path.join(d, "args.json")
        write(canned, json.dumps({"contract": "pulso.outcome.v1", "cells": []}))
        r = subprocess.run([sys.executable, str(HERE / "outcome_double.py"), "--canned", canned, "--log", log, "--cells", "a", "--treated", "b", "--release-date", "2025-01", "--window-months", "3"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout)["contract"], "pulso.outcome.v1")
        self.assertEqual(json.loads(Path(log).read_text())["release_date"], "2025-01")


if __name__ == "__main__":
    unittest.main()
