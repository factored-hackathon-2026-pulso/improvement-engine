import unittest

from claude_standin import ed0_detect as ed0


def sig(metric, num, dig, ref=None):
    return {"metric_id": metric, "numerator": num, "denominator": 30, "minimum_support": 5,
            "digest": dig, "pattern_ref": ref}


class Project(unittest.TestCase):
    def test_projection_is_aggregate_only_and_records_discards(self):
        main = sig(ed0.FAMILY, 22, "d3", "sha256:" + "a" * 64)
        r = {"signal": main, "run_id": "r1", "e0_recurrence_holdout": {"status": "replicated"},
             "signals": [sig("e0_technical_error_rate", 0, "d1"), sig("e0_tool_retry_case_rate", 0, "d2"), main]}
        d = ed0.project(r)
        self.assertEqual([x["metric_id"] for x in d["discards"]], ["e0_technical_error_rate", "e0_tool_retry_case_rate"])
        self.assertEqual((d["data_origin"], d["providers"]), ("generated_sample", ["local"]))
        self.assertNotIn("rows", d)

    def test_unexpected_family_rejected(self):
        with self.assertRaises(ValueError):
            ed0.project({"signal": sig("other", 1, "d"), "signals": [], "run_id": "r"})


if __name__ == "__main__":
    unittest.main()
