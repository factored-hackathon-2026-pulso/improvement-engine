import copy
import unittest

from roleplay_llm.scanner import SCANNER_ID, scan_payload, suppress_rows


def row(i=0, count=25):
    return {"metric_id": "resolution_rate", "window_id": "w_2026_09", "count": count, "rate": 0.42,
            "g_segment": "a1b2c3d4e5f60718", "evidence_ref": f"ev_{i:08x}"}


def payload(rows=None, **over):
    p = {
        "goal": "Find the signal family with the largest drop and cite evidence.",
        "inputs": {"family_id": "fam_001", "window_id": "w_2026_09"},
        "step": 1,
        "tools": [{"tool": "pulso/lab_query@1.0.0", "description": "Query treated aggregates.",
                   "args_schema": {"type": "object", "properties": {"metric_id": {"type": "string"}}}}],
        "observations": [{"tool": "pulso/lab_query@1.0.0", "args": {"metric_id": "resolution_rate"},
                          "status": "ok", "result": {"rows": rows if rows is not None else [row()]},
                          "error": None}],
        "feedback": None,
        "output_schema": {"type": "object", "properties": {"summary": {"type": "string"}}},
    }
    p.update(over)
    return p


class TreatedPayloadScanner(unittest.TestCase):
    def test_lab_row_with_free_text_field_is_rejected(self):
        bad = row()
        bad["note"] = "customer said the refund never arrived"
        r = scan_payload(payload([bad]))
        self.assertFalse(r.ok)
        self.assertTrue(any("note" in v for v in r.violations))

    def test_k_anonymous_aggregate_passes(self):
        r = scan_payload(payload())
        self.assertTrue(r.ok, r.violations)
        self.assertEqual(r.scanner_id, SCANNER_ID)

    def test_hundred_treated_payloads_pass(self):
        for i in range(100):
            r = scan_payload(payload([row(i, 10 + i)], step=i % 5 + 1))
            self.assertTrue(r.ok, (i, r.violations))

    def test_unknown_top_level_key_rejected(self):
        p = payload()
        p["raw_conversation"] = "x"
        self.assertFalse(scan_payload(p).ok)

    def test_raw_rows_and_conversation_texts_rejected(self):
        raw = [
            {"conversation_id": "c1", "text": "Hola, necesito ayuda con mi pedido"},
            {"email": "a@b.com", "count": 30},
            {"customer": "Maria Perez", "count": 30},
            {"metric_id": "m", "window_id": "w", "count": 30, "rate": 0.5, "g_segment": "zz",
             "evidence_ref": "ev_00000001"},
            {"metric_id": "m", "window_id": "w", "count": 30, "rate": 0.123456, "g_segment": "a1b2c3d4e5f60718",
             "evidence_ref": "ev_00000001"},
            {"metric_id": "m", "window_id": "w", "count": 3, "rate": 0.5, "g_segment": "a1b2c3d4e5f60718",
             "evidence_ref": "ev_00000001"},
            {"metric_id": "has spaces here", "window_id": "w", "count": 30, "rate": 0.5,
             "g_segment": "a1b2c3d4e5f60718", "evidence_ref": "ev_00000001"},
            {"metric_id": "m", "window_id": "w", "count": 30, "rate": 0.5, "g_segment": "a1b2c3d4e5f60718",
             "evidence_ref": "see /home/user/file.csv"},
        ]
        for r in raw:
            self.assertFalse(scan_payload(payload([r])).ok, r)
        for text in ["Hola, necesito ayuda", "mail me at a@b.com", "5551234567890"]:
            p = payload()
            p["inputs"] = {"family_id": text}
            self.assertFalse(scan_payload(p).ok, text)
            p = payload()
            p["observations"][0]["args"] = {"q": text}
            self.assertFalse(scan_payload(p).ok, text)
        p = payload()
        p["observations"][0]["result"] = {"text": "raw conversation"}
        self.assertFalse(scan_payload(p).ok)
        p = payload()
        p["observations"][0]["error"] = "free text error with spaces"
        self.assertFalse(scan_payload(p).ok)

    def test_below_k_row_is_suppressed_and_suppressed_passes(self):
        rows = suppress_rows([row(1, 3), row(2, 25)], k=10)
        self.assertEqual(rows[0]["count"], "<k")
        self.assertNotIn("rate", rows[0])
        self.assertNotIn("g_segment", rows[0])
        self.assertEqual(rows[1]["count"], 25)
        self.assertTrue(scan_payload(payload(rows)).ok)
        self.assertFalse(scan_payload(payload([row(1, 3)])).ok)

    def test_k_is_configurable(self):
        self.assertFalse(scan_payload(payload([row(1, 15)]), k=20).ok)
        self.assertTrue(scan_payload(payload([row(1, 15)]), k=10).ok)

    def test_scan_does_not_mutate_and_is_deterministic(self):
        p = payload()
        before = copy.deepcopy(p)
        a, b = scan_payload(p), scan_payload(p)
        self.assertEqual(p, before)
        self.assertEqual(a, b)

    def test_non_dict_and_bad_types_rejected(self):
        self.assertFalse(scan_payload([]).ok)
        self.assertFalse(scan_payload(payload(step="1")).ok)
        self.assertFalse(scan_payload(payload(feedback="x" * 600)).ok)


if __name__ == "__main__":
    unittest.main()
