"""AG2 tests: digital error rate (M7), non-consenting sends (M8), transaction declines (M9), handled-time share (M10).

Synthetic fixtures only. Covers the k / (valid,pos) masking rule, the no-margin rule, anonymous and partial-month
exclusions, and that nothing identifying or free-text reaches the output.
"""
import csv
import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE))
import bank_cells as bc  # noqa: E402
from test_bank_cells import CALL_COLS as _CALL_COLS, synthetic_root, write_table  # noqa: E402

CALL_COLS = _CALL_COLS + ["duration_seconds"]
DIG_COLS = ["event_date", "customer_id", "event_type", "channel", "action", "ip_address", "page_title"]
SND_COLS = ["send_date", "send_id", "campaign_id", "customer_id", "send_channel", "subject"]
TRX_COLS = ["transaction_date", "transaction_id", "customer_id", "channel", "transaction_status", "merchant_name"]
CUS_COLS = ["customer_id", "document_number", "email", "segment", "accepts_marketing"]
CAM_COLS = ["campaign_id", "campaign_name", "campaign_type"]


def write_ref(root: Path, name: str, cols, rows):
    with open(root / name, "w", encoding="utf-8-sig", newline="") as h:
        w = csv.writer(h)
        w.writerow(cols)
        w.writerows(rows)


def ag2_root(tmp: Path):
    """40 customers: even ids Plus + consenting, odd ids Basic + non-consenting. Events in Feb 2024 (+ one partial month)."""
    cus = [[f"CLI-{i:03d}", f"DOC{i}", f"u{i}@example.org", ["Plus", "Basic"][i % 2], "False" if i % 2 else "True"]
           for i in range(40)]
    write_ref(tmp, "customers.csv", CUS_COLS, cus)
    write_ref(tmp, "marketing_campaigns.csv", CAM_COLS, [["CMP-A", "n", "Push"], ["CMP-B", "n", "Promoción"]])
    dig, snd, trx, calls = [], [], [], []
    for i in range(400):
        cid = f"CLI-{i % 40:03d}"
        dig.append(["2024-02-05 10:00:00", cid, "Error" if i % 4 == 0 else "Click", "App", "initiate_transfer", "1.2.3.4", "T"])
        dig.append(["2024-02-05 10:00:00", cid, "Error" if i % 20 == 0 else "Click", "App", "view_home", "1.2.3.4", "T"])
        dig.append(["2024-02-05 10:00:00", cid, "Click", "App", "login", "1.2.3.4", "T"])
        dig.append(["2024-02-05 10:00:00", "", "Error", "App", "initiate_transfer", "1.2.3.4", "T"])
        dig.append(["2023-06-20 10:00:00", cid, "Error", "App", "initiate_transfer", "1.2.3.4", "T"])
        snd.append(["2024-02-05 10:00:00", f"SND-{i}", "CMP-A" if i % 2 else "CMP-B", cid, "Email", "free text"])
        snd.append(["2024-02-05 10:00:00", f"SND-X{i}", "CMP-A", "CLI-UNKNOWN", "Email", "free text"])
        trx.append(["2024-02-05 10:00:00", f"TRX-{i}", cid, "Web", "Declined" if i % 10 == 0 else "Approved", "Shop"])
        calls.append(["2024-02-05 10:00:00", f"INT-{i}", cid, "Phone", "Queja", "False" if i % 2 else "True", "600"])
    calls.append(["2024-02-05 10:00:00", "INT-NODUR", "CLI-001", "Phone", "Queja", "False", ""])
    write_table(tmp, "digital_events", DIG_COLS, dig)
    write_table(tmp, "campaign_sends", SND_COLS, snd)
    write_table(tmp, "transactions", TRX_COLS, trx)
    write_table(tmp, "call_center_interactions", CALL_COLS, calls)
    return tmp


class Ag2MetricTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = ag2_root(Path(self.tmp.name))
        self.rows, self.stats = bc.build(self.root, k=1)
        self.all_rows = self.rows
        self.rows = [r for r in self.rows if bc.is_month(r["period"])]  # month cells; ALL/W rows: FullPeriodTests

    def tearDown(self):
        self.tmp.cleanup()

    def total(self, metric, dims=None):
        sel = [r for r in self.rows if r["metric"] == metric and (dims is None or r["dims"] == dims)]
        return sum(r["numerator"] for r in sel), sum(r["denominator"] for r in sel)

    def test_m7_error_share_by_action_and_channel_excludes_anonymous_structural_and_partial(self):
        self.assertEqual(self.total("M7", {"action": "initiate_transfer", "channel": "App"}), (100, 400))
        self.assertEqual(self.total("M7", {"action": "view_home", "channel": "App"}), (20, 400))
        self.assertEqual({r["dims"]["action"] for r in self.rows if r["metric"] == "M7"}, {"initiate_transfer", "view_home"})
        self.assertEqual(self.stats["digital_events"]["anonymous_excluded"], 400)
        self.assertEqual(self.stats["digital_events"]["structural_actions_excluded"], 400)

    def test_m8_nonconsenting_sends_by_campaign_type_and_channel_known_consent_only(self):
        self.assertEqual(self.total("M8", {"campaign_type": "Push", "channel": "Email"}), (200, 200))
        self.assertEqual(self.total("M8", {"campaign_type": "Promocion", "channel": "Email"}), (0, 200))
        self.assertEqual(self.stats["campaign_sends"]["consent_unknown_excluded"], 400)

    def test_m9_decline_rate_by_channel_and_segment(self):
        self.assertEqual(self.total("M9", {"channel": "Web", "customer_segment": "Plus"}), (40, 200))
        self.assertEqual(self.total("M9", {"channel": "Web", "customer_segment": "Basic"}), (0, 200))

    def test_m10_handled_time_share_is_in_hours_and_needs_a_known_flag_and_duration(self):
        # 400 calls x 600 s = 66.67 h; unresolved = 200 x 600 s = 33.33 h; each emitted cell is floored to whole hours
        num, den = self.total("M10", {"reason_category": "Queja", "channel": "Phone"})
        self.assertLessEqual(num, den)
        self.assertTrue(30 <= num <= 34 and 62 <= den <= 67, (num, den))
        self.assertEqual({r["dims"].get("reason_category") for r in self.rows if r["metric"] == "M10"}, {"Queja"})

    def test_new_cells_carry_only_aggregates(self):
        for r in self.rows:
            self.assertEqual(set(r), {"metric", "dims", "half", "period", "numerator", "denominator"})
        blob = json.dumps(self.rows)
        for bad in ("CLI-", "INT-", "SND-", "TRX-", "CMP-", "DOC", "@example", "free text", "1.2.3.4", "Shop"):
            self.assertNotIn(bad, blob)

    def test_partial_month_rows_are_excluded_from_the_new_tables(self):
        self.assertEqual({r["period"] for r in self.rows}, {"2024-02"})
        self.assertGreater(self.stats["partial_month_rows_excluded"]["digital_events"], 0)

    def test_output_is_deterministic(self):
        again, _ = bc.build(self.root, k=1)
        self.assertEqual(bc.to_ndjson(again), bc.to_ndjson(self.all_rows))


class Ag2PrivacyRuleTests(unittest.TestCase):
    def test_pos_between_1_and_9_masks_the_cell_but_a_zero_with_enough_valid_is_kept(self):
        with tempfile.TemporaryDirectory() as t:
            root = Path(t)
            write_ref(root, "customers.csv", CUS_COLS, [[f"CLI-{i:03d}", "d", "e", "Plus", "True"] for i in range(400)])
            write_ref(root, "marketing_campaigns.csv", CAM_COLS, [])
            ev = []
            for i in range(400):
                cid = f"CLI-{i:03d}"
                ev.append(["2024-02-05 10:00:00", cid, "Error" if i < 6 else "Click", "App", "view_home", "", ""])
                ev.append(["2024-02-05 10:00:00", cid, "Click", "Web", "view_home", "", ""])
                ev.append(["2024-02-05 10:00:00", cid, "Error", "Web Chat", "view_home", "", ""])
            write_table(root, "digital_events", DIG_COLS, ev)
            rows, stats = bc.build(root, k=10)
        m7 = [r for r in rows if r["metric"] == "M7"]
        self.assertTrue(m7)
        for r in m7:
            for c in (r["numerator"], r["denominator"] - r["numerator"]):
                self.assertTrue(c == 0 or c >= 10, r)
        channels = {r["dims"]["channel"] for r in m7}
        self.assertIn("Web", channels)        # pos 0 with valid >= 10 is published
        self.assertIn("Web Chat", channels)   # pos == valid is published
        self.assertNotIn("App", channels)     # pos 1..9 per half is masked
        self.assertGreater(stats["suppressed_by_metric"].get("M7", 0), 0)

    def test_no_margin_total_is_emitted_for_new_metrics(self):
        with tempfile.TemporaryDirectory() as t:
            rows, _ = bc.build(ag2_root(Path(t)), k=1)
        for r in rows:
            if r["metric"] in ("M7", "M8", "M9", "M10"):
                self.assertTrue(r["dims"], "an all-population margin would allow differencing a suppressed cell")

    def test_reference_files_are_read_by_name_with_a_column_allowlist_and_no_pii(self):
        self.assertEqual(set(bc.REF_COLUMNS), {"customers.csv", "marketing_campaigns.csv"})
        self.assertEqual(bc.REF_COLUMNS["customers.csv"], ("customer_id", "segment", "accepts_marketing"))
        for cols in bc.REF_COLUMNS.values():
            for c in cols:
                for pii in ("email", "document", "name", "phone", "address", "birth"):
                    self.assertNotIn(pii, c)

    def test_absent_optional_tables_are_skipped_not_fatal(self):
        with tempfile.TemporaryDirectory() as t:
            rows, stats = bc.build(synthetic_root(Path(t)))
        self.assertTrue(rows)
        self.assertIn("digital_events", stats["tables_absent"])

    def test_everything_is_hidden_when_k_cannot_be_met(self):
        with tempfile.TemporaryDirectory() as t:
            rows, _ = bc.build(ag2_root(Path(t)), k=10_000)
        self.assertEqual(rows, [])


if __name__ == "__main__":
    unittest.main()
