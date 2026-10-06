"""Contract of the loader's automatic cells export (infra `run-bank-cells.sh`, journal 0672): the engine image carries `bank_cells.py`
and the loader runs it, in a container with no network and no credentials, over the landing CSV layout.

Synthetic fixtures only (the AG2 synthetic root); no bank data is read. Pinned here so a change of the aggregator's inputs or of the image
fails in this repo before the infra loader meets it:
  - the Dockerfile copies the aggregator to the path the loader runs;
  - the aggregator is stdlib only (the image has python3, not pip packages);
  - ALLOWED_TABLES / REF_COLUMNS are the eight inputs the loader syncs from `landing/bank/`;
  - the CLI the loader invokes writes cells that pass the loader's k>=10 gate (same rules as infra `check_cells_k.py`).
"""
import ast
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE))
import bank_cells as bc  # noqa: E402
from test_bank_cells_ag2 import ag2_root  # noqa: E402

REPO = HERE.parents[2]
IMAGE_PATH = "/opt/pulso/aggregate/bank_cells.py"
LOADER_TABLES = ("call_center_interactions", "complaints", "satisfaction_surveys", "digital_events", "campaign_sends", "transactions")
LOADER_REFS = ("customers.csv", "marketing_campaigns.csv")
GATE_KEYS = {"metric", "dims", "half", "period", "numerator", "denominator"}  # infra check_cells_k.py ALLOWED


def loader_gate(lines, k_min=10):
    """Same rules as infra `check_cells_k.py` (kept in sync by hand; the infra repo tests the real one)."""
    bad = rows = 0
    for raw in lines:
        if not raw.strip():
            continue
        rows += 1
        cell = json.loads(raw)
        n, d = cell.get("numerator"), cell.get("denominator")
        ok = (isinstance(cell, dict) and not (set(cell) - GATE_KEYS) and {"metric", "numerator", "denominator"} <= set(cell)
              and isinstance(n, int) and isinstance(d, int) and 0 <= n <= d and d >= k_min)
        bad += 0 if ok else 1
    return rows, bad


class ImageContract(unittest.TestCase):
    def test_dockerfile_copies_only_the_aggregator_to_the_path_the_loader_runs(self):
        text = (REPO / "Dockerfile").read_text(encoding="utf-8")
        self.assertIn(f"COPY scripts/aggregate/bank_cells.py {IMAGE_PATH}", text)
        self.assertNotIn("COPY scripts/aggregate ", text, "only the one stdlib file, not tests or the other aggregators")

    def test_the_aggregator_imports_only_the_standard_library(self):
        tree = ast.parse((HERE.parent / "bank_cells.py").read_text(encoding="utf-8"))
        mods = {n.names[0].name.split(".")[0] for n in ast.walk(tree) if isinstance(n, ast.Import)}
        mods |= {n.module.split(".")[0] for n in ast.walk(tree) if isinstance(n, ast.ImportFrom) and n.module}
        self.assertTrue(mods <= set(sys.stdlib_module_names), mods - set(sys.stdlib_module_names))

    def test_the_inputs_are_the_eight_the_loader_syncs(self):
        self.assertEqual(tuple(bc.ALLOWED_TABLES), LOADER_TABLES)
        self.assertEqual(tuple(bc.REF_COLUMNS), LOADER_REFS)
        for cols in bc.REF_COLUMNS.values():
            self.assertFalse({"email", "document_number", "name", "phone"} & set(cols), "reference files are read by column allowlist, no PII")


class LoaderInvocation(unittest.TestCase):
    def test_cli_as_the_loader_runs_it_writes_cells_that_pass_the_gate(self):
        with tempfile.TemporaryDirectory() as td:
            (Path(td) / "in").mkdir()
            root = ag2_root(Path(td) / "in")  # landing layout: <table>/year=.../x.csv plus the two root files
            out = Path(td) / "out" / "cells.ndjson"
            r = subprocess.run([sys.executable, str(HERE.parent / "bank_cells.py"), "--data-root", str(root), "--out", str(out), "--k", "10"],
                               capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            lines = out.read_text(encoding="utf-8").splitlines()
            rows, bad = loader_gate(lines)
            self.assertGreater(rows, 0)
            self.assertEqual(bad, 0, "every published cell passes the k>=10 gate with allowed keys only")
            blob = out.read_text(encoding="utf-8") + r.stdout
            for needle in ("CLI-", "INT-", "SND-", "TRX-", "@example.org", "DOC1"):
                self.assertNotIn(needle, blob, "no identifiers in the cells or in the printed summary")
            stats = json.loads(r.stdout)
            self.assertEqual(stats["k"], 10)
            self.assertEqual(sorted(stats["tables_absent"]), ["complaints", "satisfaction_surveys"], "the AG2 fixture has no such tables; absent tables are reported")

    def test_missing_tables_are_reported_so_the_loader_can_refuse_the_run(self):
        with tempfile.TemporaryDirectory() as td:
            _, stats = bc.build(Path(td), k=10)
            self.assertEqual(sorted(stats["tables_absent"]), sorted(LOADER_TABLES))


if __name__ == "__main__":
    unittest.main()
