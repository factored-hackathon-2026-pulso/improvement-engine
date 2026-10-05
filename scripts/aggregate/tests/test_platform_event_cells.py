"""EVT1 tests: platform events -> cells. Synthetic hand-built events only (no platform data)."""
import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import platform_event_cells as pec  # noqa: E402


class Log:
    """Builds exporter-shaped events and cases."""

    def __init__(self):
        self.events, self.cases, self.seq, self.n = [], [], 0, 0

    def case(self, channel="app_chat", language="es", case_type="app_issue", customer=None, day=1):
        self.n += 1
        cid = f"CASE-{self.n:05d}"
        self.cases.append({"case_id": cid, "customer_id": customer or f"CUS-{self.n:05d}", "channel": channel,
                           "language": language, "case_type": case_type, "opened_at": f"2026-09-{day:02d}T09:00:00Z"})
        return cid

    def ev(self, cid, etype, payload=None, day=1):
        self.seq += 1
        self.events.append({"sequence": self.seq, "event_type": etype, "case_id": cid,
                            "event_time": f"2026-09-{day:02d}T10:00:00Z", "payload": payload or {}})

    def decided(self, cid, decision, permille=None, release="rel-a", agent="copiloto-asesor@1.0.0", day=1):
        self.ev(cid, "copilot.suggestion_decided", {"subject": "reply", "decision": decision, "edit_distance_permille": permille,
                                                    "release": release, "agent": agent}, day)


def rows_of(log, k=10):
    rows, stats = pec.build(log.events, log.cases, k=k)
    return rows, stats


def find(rows, metric, dims, half=None, period="ALL"):
    return [r for r in rows if r["metric"] == metric and r["dims"] == dims and r["period"] == period and (half is None or r["half"] == half)]


def total(rows, metric, dims, period="ALL"):
    rs = find(rows, metric, dims, None, period)
    return sum(r["numerator"] for r in rs), sum(r["denominator"] for r in rs)


class Counting(unittest.TestCase):
    def test_draft_reject_counts_discarded_and_ignored_over_the_four_reply_decisions(self):
        log = Log()
        plan = ["used"] * 40 + ["edited"] * 30 + ["discarded"] * 50 + ["ignored"] * 20
        for d in plan:
            log.decided(log.case(), d, 100 if d in ("used", "edited") else None)
        rows, _ = rows_of(log)
        self.assertEqual(total(rows, "P_DRAFT_REJECT", {"case_type": "app_issue", "channel": "app_chat"}), (70, 140))
        self.assertEqual(total(rows, "P_DRAFT_REJECT", {"language": "es", "channel": "app_chat"}), (70, 140))

    def test_escalation_decisions_and_accepted_are_not_reply_drafts(self):
        log = Log()
        for _ in range(30):
            c = log.case()
            log.decided(c, "used", 10)
            log.ev(c, "copilot.suggestion_decided", {"subject": "escalation", "decision": "accepted"})
        rows, stats = rows_of(log)
        self.assertEqual(total(rows, "P_DRAFT_REJECT", {"case_type": "app_issue", "channel": "app_chat"}), (0, 30))
        self.assertEqual(stats["discards"]["decision_not_a_reply_draft"], 30)

    def test_heavy_edit_bucket_is_an_aggregate_share_of_used_and_edited_with_a_distance(self):
        log = Log()
        for p in [100] * 30 + [499] * 10 + [500] * 12 + [900] * 18:
            log.decided(log.case(), "edited", p)
        for _ in range(15):
            log.decided(log.case(), "discarded")  # no distance, not in this metric
        rows, _ = rows_of(log)
        self.assertEqual(total(rows, "P_DRAFT_HEAVY_EDIT", {"case_type": "app_issue", "channel": "app_chat"}), (30, 70))

    def test_suggestion_none_and_failed_rates(self):
        log = Log()
        for i in range(1000):
            c = log.case()
            log.ev(c, "copilot.suggestion_ready" if i < 800 else "copilot.suggestion_none", {"agent": "copiloto-asesor@1.0.0", "release": "rel-a"})
        for _ in range(250):
            log.ev(log.case(), "copilot.suggestion_failed", {"failure_code": "gateway_timeout"})
        rows, _ = rows_of(log)
        self.assertEqual(total(rows, "P_SUGG_NONE", {"case_type": "app_issue", "channel": "app_chat"}), (200, 1000))
        # failed is over ready + none + failed
        self.assertEqual(total(rows, "P_SUGG_FAILED", {"channel": "app_chat", "language": "es"}), (250, 1250))

    def test_tool_mix_is_a_case_share_with_a_zero_numerator_published_only_over_k(self):
        log = Log()
        for i in range(600):
            c = log.case()
            log.ev(c, "copilot.suggestion_ready", {"agent": "copiloto-asesor@1.0.0", "release": "rel-a"})
            if i < 300:
                log.ev(c, "copilot.tool_used", {"tool": "consultar_cargos"})
            if i < 120:
                log.ev(c, "copilot.tool_used", {"tool": "estado_pqr"})
        rows, _ = rows_of(log)
        self.assertEqual(total(rows, "P_TOOL_USE", {"case_type": "app_issue", "tool": "consultar_cargos"}), (300, 600))
        self.assertEqual(total(rows, "P_TOOL_USE", {"case_type": "app_issue", "tool": "estado_pqr"}), (120, 600))

    def test_type_reassignment_is_a_change_away_from_a_real_type_not_the_first_labelling(self):
        log = Log()
        for i in range(600):  # 200 corrected from undue_charge, the rest untouched
            c = log.case(case_type="app_issue" if i < 200 else "undue_charge")
            if i < 200:
                log.ev(c, "case.type_changed", {"from": "undue_charge", "to": "app_issue"})
        for _ in range(400):  # first labelling none -> app_issue is not a reassignment and not in the denominator as a real type start
            c = log.case(case_type="app_issue")
            log.ev(c, "case.type_changed", {"from": "none", "to": "app_issue"})
        rows, _ = rows_of(log)
        self.assertEqual(total(rows, "P_TYPE_REASSIGN", {"case_type": "undue_charge", "channel": "app_chat"}), (200, 600))
        self.assertEqual(total(rows, "P_TYPE_REASSIGN", {"case_type": "app_issue", "channel": "app_chat"}), (0, 400))

    def test_assistant_escalation_rate_uses_the_release_of_the_turn_answers(self):
        log = Log()
        for i in range(600):
            c = log.case(channel="web_chat")
            log.ev(c, "assistant.turn_answered", {"agent": "recepcion@1.0.0", "release": "rel-a" if i < 400 else "rel-b"})
            log.ev(c, "assistant.ended", {"result": "escalated" if i % 3 == 0 else "resolved"})
        rows, _ = rows_of(log)
        self.assertEqual(total(rows, "P_ASSIST_ESCALATION", {"channel": "web_chat", "case_type": "app_issue"}), (200, 600))
        self.assertEqual(total(rows, "P_ASSIST_ESCALATION", {"release": "rel-a", "agent": "recepcion@1.0.0"})[1], 400)

    def test_old_channel_names_are_one_channel(self):
        log = Log()
        for i in range(40):
            log.decided(log.case(channel="chat_app" if i % 2 else "app_chat"), "discarded")
        rows, _ = rows_of(log)
        self.assertEqual(total(rows, "P_DRAFT_REJECT", {"case_type": "app_issue", "channel": "app_chat"}), (40, 40))
        self.assertEqual(find(rows, "P_DRAFT_REJECT", {"case_type": "app_issue", "channel": "chat_app"}), [])


class Windows(unittest.TestCase):
    def test_all_equals_w1_plus_w2_per_half_when_published(self):
        log = Log()
        for i in range(400):
            day = 1 if i < 200 else 28
            log.decided(log.case(day=day), "discarded" if i % 3 else "used", 50, day=day)
        rows, _ = rows_of(log)
        for half in ("discovery", "holdout"):
            d = {"case_type": "app_issue", "channel": "app_chat"}
            a = find(rows, "P_DRAFT_REJECT", d, half, "ALL")[0]
            w1 = find(rows, "P_DRAFT_REJECT", d, half, "W1")[0]
            w2 = find(rows, "P_DRAFT_REJECT", d, half, "W2")[0]
            self.assertEqual((w1["numerator"] + w2["numerator"], w1["denominator"] + w2["denominator"]), (a["numerator"], a["denominator"]))

    def test_only_all_w1_w2_periods_exist(self):
        log = Log()
        for i in range(100):
            log.decided(log.case(day=1 + i % 28), "discarded", day=1 + i % 28)
        rows, _ = rows_of(log)
        self.assertEqual({r["period"] for r in rows}, {"ALL", "W1", "W2"})

    def test_halves_follow_the_customer_so_cases_of_one_customer_stay_together(self):
        log = Log()
        for i in range(60):
            for _ in range(2):
                log.decided(log.case(customer=f"CUS-{i:03d}"), "discarded")
        rows, _ = rows_of(log)
        halves = {r["half"] for r in rows}
        self.assertEqual(halves, {"discovery", "holdout"})
        # even customer counts per half: every customer's two cases land in one half
        for r in find(rows, "P_DRAFT_REJECT", {"case_type": "app_issue", "channel": "app_chat"}):
            self.assertEqual(r["denominator"] % 2, 0)


class Suppression(unittest.TestCase):
    def test_k_rule_hides_small_numerator_and_complement(self):
        log = Log()
        for i in range(200):
            log.decided(log.case(case_type="app_issue"), "discarded" if i < 7 else "used", 10)  # 7 of 200
            log.decided(log.case(case_type="undue_charge"), "discarded" if i < 100 else "used", 10)
        rows, _ = rows_of(log)
        self.assertTrue(rows)
        for r in rows:
            n, d = r["numerator"], r["denominator"]
            self.assertTrue(d >= 10 and (n == 0 or n >= 10) and (d - n == 0 or d - n >= 10), r)
        # app_issue has 7 discarded in total: at best one half holds all of them, and each published half is 0 or >= 10
        small = find(rows, "P_DRAFT_REJECT", {"case_type": "app_issue", "channel": "app_chat"})
        for r in small:
            self.assertNotIn(r["numerator"], range(1, 10))

    def test_no_margin_rows_every_row_has_both_dims_of_its_signature(self):
        log = Log()
        for i in range(300):
            log.decided(log.case(language="pt" if i % 2 else "es"), "discarded" if i % 4 else "used", 30)
        rows, _ = rows_of(log)
        sigs = {tuple(sorted(r["dims"])) for r in rows}
        for s in sigs:
            self.assertEqual(len(s), 2, s)

    def test_output_has_no_ids_no_text_and_only_closed_dims(self):
        log = Log()
        for i in range(120):
            c = log.case()
            log.ev(c, "copilot.suggestion_decided", {"subject": "reply", "decision": "discarded", "edit_distance_permille": None,
                                                     "release": "rel-a", "agent": "copiloto-asesor@1.0.0",
                                                     "note": "free text with CASE-00001 and a name", "text": "hola Maria"})
        text = pec.to_ndjson(rows_of(log)[0])
        for needle in ("CASE-", "CUS-", "free text", "Maria", "hola", "note"):
            self.assertNotIn(needle, text)
        for line in text.splitlines():
            self.assertEqual(set(json.loads(line)), {"metric", "dims", "half", "period", "numerator", "denominator"})

    def test_a_dimension_value_that_is_free_text_or_an_id_is_a_named_discard_never_a_cell(self):
        log = Log()
        for i in range(80):
            log.decided(log.case(), "discarded", release="please call Ana" if i % 2 else "CASE-00007")
        rows, stats = rows_of(log)
        self.assertFalse([r for r in rows if "release" in r["dims"]])
        self.assertEqual(stats["discards"]["unsafe_payload_value:copilot.suggestion_decided:release"], 80)

    def test_discard_counts_below_k_are_published_as_null(self):
        log = Log()
        for _ in range(60):
            log.decided(log.case(), "discarded")
        log.ev("CASE-99999", "copilot.suggestion_failed", {})  # event whose case is not in cases
        _, stats = rows_of(log)
        self.assertIsNone(stats["discards"]["event_case_not_in_cases"])

    def test_unknown_dimension_units_are_dropped_not_pooled_into_an_unknown_cell(self):
        log = Log()
        for _ in range(60):
            log.decided(log.case(), "discarded", release=None)  # no release
        rows, stats = rows_of(log)
        self.assertFalse([r for r in rows if "release" in r["dims"]])
        self.assertTrue([r for r in rows if r["dims"].get("channel") == "app_chat"])
        self.assertEqual(stats["discards"]["dimension_unknown:P_DRAFT_REJECT"], 60)

    def test_building_twice_is_deterministic(self):
        log = Log()
        for i in range(200):
            log.decided(log.case(), "discarded" if i % 3 else "used", 40)
        self.assertEqual(pec.to_ndjson(rows_of(log)[0]), pec.to_ndjson(rows_of(log)[0]))


if __name__ == "__main__":
    unittest.main()
