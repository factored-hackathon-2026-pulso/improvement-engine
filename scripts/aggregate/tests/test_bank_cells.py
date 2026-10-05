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
        rows = [r for r in rows if bc.is_month(r["period"])]  # full-period / window rows: FullPeriodTests
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
        self.assertTrue({"ALL", "W1"} <= {r["period"] for r in rows})
        rows = [r for r in rows if bc.is_month(r["period"])]
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
                              and r["period"] == r3["period"] and "reason_category" in r["dims"]
                              and r["dims"].get("channel") == r3["dims"]["channel"])
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


class FullPeriodTests(unittest.TestCase):
    """DET1: full-period (ALL) and window (W1/W2) cells, k on the pooled cell, no differencing leaks across levels."""

    @staticmethod
    def raw(spec):
        """spec: {(reason, channel|None, half, period): (num, den)} for metric M1."""
        out = {}
        for (reason, chan, half, period), v in spec.items():
            dims = {"reason_category": reason, **({"channel": chan} if chan else {})}
            out[("M1", tuple(sorted(dims.items())), half, period)] = list(v)
        return out

    @staticmethod
    def find(rows, reason, chan, half, period):
        want = {"reason_category": reason, **({"channel": chan} if chan else {})}
        for r in rows:
            if r["metric"] == "M1" and r["dims"] == want and r["half"] == half and r["period"] == period:
                return r
        return None

    def test_pooled_cell_is_published_even_when_every_month_is_below_k(self):
        months = [f"2024-{m:02d}" for m in range(1, 13)]
        spec = {("Retencion", "Web", "discovery", m): (4, 9) for m in months}  # 4/9 per month: all suppressed
        rows, _, _ = bc.publish(self.raw(spec), 10)
        all_row = self.find(rows, "Retencion", "Web", "discovery", "ALL")
        self.assertEqual((all_row["numerator"], all_row["denominator"]), (48, 108))
        self.assertEqual([r for r in rows if bc.is_month(r["period"])], [])

    def test_full_period_cell_still_obeys_k_on_numerator_complement_and_denominator(self):
        spec = {("Retencion", "Web", "discovery", "2024-01"): (4, 200)}  # numerator 4 < k even pooled
        self.assertEqual(bc.publish(self.raw(spec), 10)[0], [])
        spec = {("Retencion", "Web", "discovery", "2024-01"): (198, 200)}  # complement 2 < k
        self.assertEqual(bc.publish(self.raw(spec), 10)[0], [])

    def test_window_rows_are_withheld_when_a_sibling_window_is_suppressed(self):
        spec = {("Tecnico", "App", "discovery", "2024-01"): (50, 200), ("Tecnico", "App", "discovery", "2025-02"): (4, 200)}
        rows, _, _ = bc.publish(self.raw(spec), 10)
        # ALL = W1 + W2 and W2 (4/200) is suppressed: W1 would reveal it by differencing, so W1 and the months are withheld
        periods = {r["period"] for r in rows if r["dims"].get("channel") == "App"}
        self.assertEqual(periods, {"ALL"})

    def test_months_withheld_when_one_suppressed_month_would_be_recovered_from_the_window(self):
        spec = {("Tecnico", "App", "discovery", "2024-01"): (50, 200), ("Tecnico", "App", "discovery", "2024-02"): (4, 200),
                ("Tecnico", "App", "discovery", "2025-01"): (50, 200)}
        rows, _, _ = bc.publish(self.raw(spec), 10)
        got = {r["period"] for r in rows if r["dims"].get("channel") == "App"}
        self.assertEqual(got, {"ALL", "W1", "W2", "2025-01"})  # W1 months withheld, W2 month fine

    def test_reason_only_parent_is_hidden_when_a_single_child_is_suppressed(self):
        spec = {("Retencion", "Web", "discovery", "2024-01"): (4, 200), ("Retencion", "App", "discovery", "2024-01"): (60, 200),
                ("Retencion", None, "discovery", "2024-01"): (64, 400)}
        rows, _, _ = bc.publish(self.raw(spec), 10)
        self.assertIsNone(self.find(rows, "Retencion", None, "discovery", "ALL"))
        self.assertIsNotNone(self.find(rows, "Retencion", "App", "discovery", "ALL"))

    def test_reason_only_parent_is_published_when_children_are_all_published(self):
        spec = {("Retencion", "Web", "discovery", "2024-01"): (40, 200), ("Retencion", "App", "discovery", "2024-01"): (60, 200),
                ("Retencion", None, "discovery", "2024-01"): (100, 400)}
        rows, _, _ = bc.publish(self.raw(spec), 10)
        self.assertIsNotNone(self.find(rows, "Retencion", None, "discovery", "ALL"))

    def test_published_levels_are_consistent_randomised(self):
        import random
        rnd = random.Random(7)
        spec = {}
        for reason in ("A", "B", "C"):
            for chan in ("X", "Y"):
                for m in [f"2024-{i:02d}" for i in range(1, 13)] + ["2025-01", "2025-02"]:
                    den = rnd.choice([8, 30, 200])
                    spec[(reason, chan, "discovery", m)] = (min(den, rnd.choice([0, 2, 9, 15, den // 2])), den)
        raw = self.raw(spec)
        for (metric, d, h, p), (n, dn) in list(raw.items()):
            parent = ("M1", (("reason_category", dict(d)["reason_category"]),), h, p)
            c = raw.setdefault(parent, [0, 0])
            c[0] += n
            c[1] += dn
        rows, _, _ = bc.publish(raw, 10)
        pub = {(tuple(sorted(r["dims"].items())), r["half"], r["period"]): (r["numerator"], r["denominator"]) for r in rows}

        def ok(n, dn):
            return dn >= 10 and (n == 0 or n >= 10) and (dn - n == 0 or dn - n >= 10)

        for (dims, half, period), (n, dn) in pub.items():
            self.assertTrue(ok(n, dn))
        # parent minus its published children: nothing, or a residual that itself passes k
        for (dims, half, period), (n, dn) in pub.items():
            dd = dict(dims)
            if "channel" in dd:
                continue
            kids = [v for (d2, h2, p2), v in pub.items()
                    if h2 == half and p2 == period and dict(d2).get("reason_category") == dd["reason_category"] and "channel" in dict(d2)]
            rn, rd = n - sum(v[0] for v in kids), dn - sum(v[1] for v in kids)
            self.assertTrue(rd == 0 or ok(rn, rd), (dims, period, rn, rd))
        # ALL minus published windows / windows minus published months: nothing, or a residual that passes k
        for (dims, half, period), (n, dn) in pub.items():
            if period not in ("ALL", "W1", "W2"):
                continue
            kids = [v for (d2, h2, p2), v in pub.items() if d2 == dims and h2 == half and
                    ((period == "ALL" and p2 in ("W1", "W2")) or (period != "ALL" and bc.is_month(p2) and bc.window_of(p2) == period))]
            rn, rd = n - sum(v[0] for v in kids), dn - sum(v[1] for v in kids)
            self.assertTrue(rd == 0 or ok(rn, rd), (dims, period, rn, rd))

    def test_level_risk_metric_gets_no_full_period_rows(self):
        raw = {("M8", (("campaign_type", "X"), ("channel", "Email")), "discovery", "2024-01"): [40, 200]}
        rows, _, _ = bc.publish(raw, 10)
        self.assertEqual({r["period"] for r in rows}, {"2024-01"})

    def test_build_emits_reason_only_m1_and_full_period_cells(self):
        with tempfile.TemporaryDirectory() as t:
            rows, stats = bc.build(synthetic_root(Path(t)), k=1)
        self.assertTrue(any(r["metric"] == "M1" and set(r["dims"]) == {"reason_category"} for r in rows))
        self.assertIn("ALL", {r["period"] for r in rows})
        self.assertIn("full_period", stats)


if __name__ == "__main__":
    unittest.main()
