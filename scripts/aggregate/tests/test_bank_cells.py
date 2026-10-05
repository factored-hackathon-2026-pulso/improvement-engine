"""Tests for bank_cells.py (aggregates only, k >= 10, deterministic customer-hash split)."""
import csv
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import bank_cells as bc  # noqa: E402

CALL_COLS = ["interaction_date", "interaction_id", "customer_id", "channel", "reason_category", "was_resolved"]
CMP_COLS = ["creation_date", "complaint_id", "customer_id", "category", "status", "sla_breached"]
SRV_COLS = ["survey_date", "survey_id", "interaction_id", "customer_id", "survey_type", "send_channel", "main_score"]


def write_table(root: Path, table: str, cols, rows):
    d = root / table / "year=2024" / "month=01" / "day=01"
    d.mkdir(parents=True, exist_ok=True)
    with open(d / f"{table}_20240101.csv", "w", encoding="utf-8-sig", newline="") as h:
        w = csv.writer(h)
        w.writerow(cols)
        w.writerows(rows)


def date(i):
    """Six full months of 2024, plus 2026-06 (partial) for i % 50 == 7."""
    return "2026-06-10 09:00:00" if i % 50 == 7 else f"2024-0{1 + i % 6}-10 09:00:00"


def synthetic_root(tmp: Path, n_cust=400):
    calls, cmps, srvs = [], [], []
    for i in range(n_cust):
        cid = f"CLI-{i:05d}"
        reason = ["Queja", "Técnico", "Comercial"][i % 3]
        chan = ["Phone", "App"][i % 2]
        resolved = "False" if (reason == "Queja" and i % 4 == 0) else "True"
        calls.append([date(i), f"INT-{i:05d}", cid, chan, reason, resolved])
        cmps.append([date(i), f"CMP-{i:05d}", cid, ["Fees", "Branch"][i % 2], ["Open", "Resolved", "Closed"][i % 3],
                     "True" if i % 5 == 0 else "False"])
        srvs.append([date(i), f"SRV-{i:05d}", f"INT-{i:05d}" if i % 5 else "", cid, ["CSAT", "NPS"][i % 2], "Email",
                     "2" if i % 6 == 0 else "5"])
    write_table(tmp, "call_center_interactions", CALL_COLS, calls)
    write_table(tmp, "complaints", CMP_COLS, cmps)
    write_table(tmp, "satisfaction_surveys", SRV_COLS, srvs)
    return tmp


class SplitTests(unittest.TestCase):
    def test_split_is_deterministic_per_customer_and_about_50_50(self):
        halves = [bc.split_half(f"CLI-{i}") for i in range(20000)]
        self.assertEqual(halves, [bc.split_half(f"CLI-{i}") for i in range(20000)])
        share = halves.count("discovery") / len(halves)
        self.assertTrue(0.48 < share < 0.52, share)


class NormaliseTests(unittest.TestCase):
    def test_accents_are_folded_to_the_closed_vocabulary(self):
        self.assertEqual(bc.norm("Técnico"), "Tecnico")
        self.assertEqual(bc.norm("Retención"), "Retencion")


class BuildTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = synthetic_root(Path(self.tmp.name))

    def tearDown(self):
        self.tmp.cleanup()

    def test_cells_have_only_aggregate_fields_no_identifiers(self):
        rows, _ = bc.build(self.root)
        self.assertTrue(rows)
        for r in rows:
            self.assertEqual(set(r), {"metric", "dims", "half", "period", "numerator", "denominator"})
            self.assertIn(r["half"], ("discovery", "holdout"))
            for v in r["dims"].values():
                self.assertFalse(v.startswith(("CLI-", "INT-", "CMP-", "SRV-")))
        blob = json.dumps(rows)
        self.assertNotIn("CLI-", blob)

    def test_k_rule_every_count_is_zero_or_at_least_ten(self):
        rows, stats = bc.build(self.root, k=10)
        for r in rows:
            n, d = r["numerator"], r["denominator"]
            self.assertGreaterEqual(d, 10)
            for c in (n, d - n):
                self.assertTrue(c == 0 or c >= 10, r)
        self.assertIn("suppressed_cells", stats)

    def test_k_violation_cells_are_suppressed_and_counted(self):
        rows, stats = bc.build(self.root, k=10_000)  # everything is below k
        self.assertEqual(rows, [])
        self.assertGreater(stats["suppressed_cells"], 0)

    def test_metric_definitions(self):
        rows, _ = bc.build(self.root, k=1)  # k=1 so the tiny synthetic table is not suppressed
        m1 = [r for r in rows if r["metric"] == "M1" and r["dims"] == {"reason_category": "Queja", "channel": "Phone"}]
        self.assertEqual(sum(r["denominator"] for r in m1), sum(1 for i in range(400) if i % 3 == 0 and i % 2 == 0 and i % 50 != 7))
        self.assertEqual(sum(r["numerator"] for r in m1), sum(1 for i in range(400) if i % 3 == 0 and i % 2 == 0 and i % 4 == 0 and i % 50 != 7))
        m4 = [r for r in rows if r["metric"] == "M4"]
        self.assertEqual(sum(r["denominator"] for r in m4), sum(1 for i in range(400) if i % 50 != 7))
        # open = Open + In Process + Escalated; here statuses Open / Resolved / Closed -> Open only
        self.assertEqual(sum(r["numerator"] for r in m4), sum(1 for i in range(400) if i % 3 == 0 and i % 50 != 7))
        self.assertTrue({r["metric"] for r in rows} >= {"M1", "M2", "M3", "M4", "M5", "M6", "M6R", "M6U"})

    def test_survey_linkage_coverage_is_reported_not_row_level(self):
        _, stats = bc.build(self.root)
        self.assertEqual(stats["surveys"]["total"], 400)
        self.assertEqual(stats["surveys"]["scored_csat"], sum(1 for i in range(400) if i % 2 == 0 and i % 50 != 7))
        self.assertEqual(stats["surveys"]["linked_scored"], sum(1 for i in range(400) if i % 2 == 0 and i % 5 and i % 50 != 7))

    def test_rows_carry_a_period_and_partial_months_are_excluded(self):
        rows, stats = bc.build(self.root, k=1)
        for r in rows:
            self.assertRegex(r["period"], r"^\d{4}-\d{2}$")
            self.assertNotIn(r["period"], ("2023-06", "2026-06"))
        self.assertGreater(stats["partial_month_rows_excluded"]["call_center_interactions"], 0)
        self.assertEqual({r["period"] for r in rows}, {f"2024-0{m}" for m in range(1, 7)})

    def test_unstratified_csat_metric_exists_for_the_dependency_flag(self):
        rows, _ = bc.build(self.root, k=1)
        self.assertIn("M6", {r["metric"] for r in rows})

    def test_output_is_deterministic_and_sorted(self):
        a, _ = bc.build(self.root)
        b, _ = bc.build(self.root)
        self.assertEqual(bc.to_ndjson(a), bc.to_ndjson(b))
        self.assertEqual(a, sorted(a, key=bc.row_key))

    def test_cli_writes_ndjson_and_summary_without_ids(self):
        out = Path(self.tmp.name) / "out" / "cells.ndjson"
        p = subprocess.run([sys.executable, str(HERE.parent / "bank_cells.py"), "--data-root", str(self.root),
                            "--out", str(out)], capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stderr)
        text = out.read_text(encoding="utf-8")
        self.assertNotIn("CLI-", text)
        self.assertNotIn("CLI-", p.stdout)
        self.assertEqual(len(text.strip().splitlines()), len(bc.build(self.root)[0]))

    def test_refuses_tables_outside_the_allowlist(self):
        with self.assertRaises(ValueError):
            bc.build(self.root, tables=("service_agents",))


class MarginDifferencingTests(unittest.TestCase):
    """A suppressed small cell must not be recoverable by subtracting published margins."""

    def make(self, tmp):
        calls, srvs = [], []
        n = 0
        # (reason, resolved flag, contacts, unresolved-or-low count) in one channel and one month
        plan = [("Queja", 200, 100), ("Tecnico", 200, 3), ("Comercial", 200, 100)]
        for reason, total, unres in plan:
            for j in range(total):
                n += 1
                res = "False" if j < unres else "True"
                calls.append(["2024-01-10 09:00:00", f"INT-{n:05d}", f"CLI-{n:05d}", "Phone", reason, res])
        # surveys: linked to Comercial contacts; resolved ones have only 3 low scores, unresolved 50 of 100
        k = 0
        for j, c in enumerate(r for r in calls if r[4] == "Comercial"):
            k += 1
            low = (c[5] == "True" and j % 70 == 0 and j >= 100) or (c[5] == "False" and j % 2 == 0)
            srvs.append(["2024-01-12 09:00:00", f"SRV-{k:05d}", c[1], c[2], "CSAT", "Email", "1" if low else "5"])
        write_table(tmp, "call_center_interactions", CALL_COLS, calls)
        write_table(tmp, "complaints", CMP_COLS, [])
        write_table(tmp, "satisfaction_surveys", SRV_COLS, srvs)
        return tmp

    def test_m3_denominator_is_not_a_difference_of_published_m1_cells(self):
        with tempfile.TemporaryDirectory() as t:
            rows, _ = bc.build(self.make(Path(t)))
        for r3 in (r for r in rows if r["metric"] == "M3"):
            hidden_free = sum(r["numerator"] for r in rows if r["metric"] == "M1" and r["half"] == r3["half"]
                              and r["period"] == r3["period"] and r["dims"]["channel"] == r3["dims"]["channel"])
            self.assertEqual(hidden_free, r3["denominator"], f"M3 minus published M1 reveals a suppressed cell: {r3}")

    def test_suppressed_m6r_or_m6u_hides_its_partner(self):
        with tempfile.TemporaryDirectory() as t:
            rows, _ = bc.build(self.make(Path(t)))
        keyed = {}
        for r in rows:
            if r["metric"] in ("M6", "M6R", "M6U"):
                keyed.setdefault((r["half"], r["period"], tuple(sorted(r["dims"].items()))), set()).add(r["metric"])
        for key, present in keyed.items():
            self.assertFalse("M6" in present and len(present & {"M6R", "M6U"}) == 1, f"M6 minus one partner reveals the other: {key} {present}")


if __name__ == "__main__":
    unittest.main()
