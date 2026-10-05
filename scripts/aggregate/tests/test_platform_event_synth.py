"""EVT1: the SYNTHETIC platform history (platform-sim backbone) through the aggregator: labelled, planted effects visible, no leaks."""
import json
import re
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import platform_event_cells as pec  # noqa: E402
import platform_event_synth as synth  # noqa: E402

_CACHE = {}


def data(n=6000, seed=7):
    if (n, seed) not in _CACHE:
        events, cases, labels = synth.generate(seed=seed, n_cases=n)
        rows, stats = pec.build(events, cases)
        _CACHE[(n, seed)] = (events, cases, labels, rows, stats)
    return _CACHE[(n, seed)]


def pooled(rows, metric, dims):
    """Sum of the published ALL cells whose dims contain `dims` (a case type across the channels that were published)."""
    rs = [r for r in rows if r["metric"] == metric and r["period"] == "ALL" and all(r["dims"].get(k) == v for k, v in dims.items())
          and set(r["dims"]) == (set(dims) | {"channel"} if "channel" not in dims else set(dims))]
    return sum(r["numerator"] for r in rs), sum(r["denominator"] for r in rs)


class Synthetic(unittest.TestCase):
    def test_the_history_is_labelled_synthetic_and_deterministic(self):
        _, _, labels, rows, _ = data()
        self.assertTrue(labels["synthetic"])
        again = synth.generate(seed=7, n_cases=6000)
        self.assertEqual(pec.to_ndjson(pec.build(again[0], again[1])[0]), pec.to_ndjson(rows))

    def test_every_family_publishes_cells(self):
        _, _, _, rows, _ = data()
        self.assertEqual({r["metric"] for r in rows}, set(pec.SIGNATURES))

    def test_planted_cells_are_visibly_above_their_same_channel_baseline_and_controls_are_flat(self):
        _, _, _, rows, _ = data()
        n, d = pooled(rows, "P_DRAFT_REJECT", {"case_type": "service_quality"})
        n0, d0 = pooled(rows, "P_DRAFT_REJECT", {"case_type": "branch_service"})
        self.assertGreater(n / d, 0.62)
        self.assertLess(abs(n0 / d0 - 0.40), 0.12)
        n, d = pooled(rows, "P_SUGG_NONE", {"case_type": "app_issue", "channel": "web_chat"})
        self.assertGreater(n / d, 0.2)

    def test_the_old_chat_channel_names_of_the_simulator_are_aliased(self):
        _, _, _, rows, _ = data()
        chans = {r["dims"]["channel"] for r in rows if "channel" in r["dims"]}
        self.assertFalse(chans & {"chat_app", "chat_web"})

    def test_no_ids_no_payload_keys_no_text_in_the_cell_table(self):
        _, _, _, rows, stats = data()
        text = pec.to_ndjson(rows) + json.dumps(stats)
        for pat in (r"CASE-", r"CUS-", r"STF-", r"STAFF", r"analyst_id", r"failure_code", r"run-", r"tr-\d", r"@example", r"hola", r"customer_id"):
            self.assertIsNone(re.search(pat, text), pat)
        for r in rows:
            self.assertTrue(set(r["dims"]) <= set(pec.DIMS))

    def test_every_published_row_is_k_safe_and_there_are_no_margin_rows(self):
        _, _, _, rows, _ = data()
        for r in rows:
            n, d = r["numerator"], r["denominator"]
            self.assertTrue(d >= 10 and (n == 0 or n >= 10) and (d - n == 0 or d - n >= 10))
            self.assertEqual(len(r["dims"]), 2)

    def test_reading_goes_through_the_exporter_allow_list_and_cannot_read_other_columns(self):
        with self.assertRaises(Exception):
            synth.policy.select_sql("event_log", ["sequence", "secret_column"])
        with self.assertRaises(Exception):
            synth.policy.select_sql("login_accounts", ["id"])


if __name__ == "__main__":
    unittest.main()
