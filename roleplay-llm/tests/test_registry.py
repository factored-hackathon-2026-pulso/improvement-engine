"""Residual-risk closure: opaque-looking free strings and free integers must not pass (deny-by-default registry)."""
import unittest

from roleplay_llm.scanner import Registry, scan_payload

from .test_scanner import payload

TOOL = {"tool": "pulso/lab_query@1.0.0", "description": "Query.",
        "args_schema": {"type": "object", "properties": {
            "metric_id": {"type": "string", "enum": ["recurrence_rate"]},
            "window_id": {"type": "string", "enum": ["w1"]}}}}


def with_inputs(**inputs):
    return payload(inputs=inputs, tools=[TOOL])


class RegistryDenyByDefault(unittest.TestCase):
    def test_name_like_identifier_is_rejected(self):
        for name in ("Maria_Gonzalez", "maria.gonzalez", "juan-perez", "maria", "Gonzalez/Maria"):
            self.assertFalse(scan_payload(with_inputs(family_id=name)).ok, name)

    def test_rut_like_values_are_rejected(self):
        for rut in ("12.345.678-5", "12345678-5", "123456785", "12345678K", "11222333-k"):
            self.assertFalse(scan_payload(with_inputs(family_id=rut)).ok, rut)

    def test_rut_like_integer_is_rejected(self):
        for n in (12345678, 9999999, 1234567, 123456):
            self.assertFalse(scan_payload(with_inputs(x=n)).ok, n)

    def test_name_like_tool_arg_is_rejected(self):
        p = with_inputs(family_id="fam_001")
        p["observations"][0]["args"] = {"metric_id": "recurrence_rate", "window_id": "Maria_Gonzalez"}
        self.assertFalse(scan_payload(p).ok)
        p["observations"][0]["args"] = {"metric_id": "12345678-5"}
        self.assertFalse(scan_payload(p).ok)

    def test_name_like_metric_or_window_in_row_is_rejected(self):
        p = with_inputs(family_id="fam_001")
        p["observations"][0]["result"]["rows"][0]["metric_id"] = "maria_gonzalez"
        self.assertFalse(scan_payload(p).ok)
        p = with_inputs(family_id="fam_001")
        p["observations"][0]["result"]["rows"][0]["window_id"] = "juan"
        self.assertFalse(scan_payload(p).ok)

    def test_group_hash_must_have_the_hash_shape_not_just_a_prefix(self):
        p = with_inputs(family_id="fam_001")
        p["observations"][0]["result"]["rows"][0]["g_group"] = "maria_gonzalez"
        self.assertFalse(scan_payload(p).ok)

    def test_expected_shapes_and_schema_values_pass(self):
        p = with_inputs(family_id="fam_001", g="g_0123456789abcdef", ev="ev_abcdef0123", n=40, window="w1",
                        metric="recurrence_rate")
        self.assertTrue(scan_payload(p).ok, scan_payload(p).violations)

    def test_registry_tokens_extend_the_allow_list_exactly(self):
        p = with_inputs(category="closing_reply_unclear")
        self.assertFalse(scan_payload(p).ok)
        self.assertTrue(scan_payload(p, registry=Registry(tokens={"closing_reply_unclear"})).ok)
        self.assertFalse(scan_payload(with_inputs(category="Closing_reply_unclear"),
                                      registry=Registry(tokens={"closing_reply_unclear"})).ok)

    def test_registry_does_not_whitelist_pii_shapes(self):
        with self.assertRaises(ValueError):
            Registry(tokens={"a@b.cl"})
        with self.assertRaises(ValueError):
            Registry(tokens={"12.345.678-5"})

    def test_input_integers_are_bounded_small(self):
        self.assertTrue(scan_payload(with_inputs(n=1000)).ok)
        self.assertFalse(scan_payload(with_inputs(n=100000)).ok)

    def test_aggregate_count_still_allowed_large(self):
        p = with_inputs(family_id="fam_001")
        p["observations"][0]["result"]["rows"][0]["count"] = 5_000_000
        self.assertTrue(scan_payload(p).ok)


if __name__ == "__main__":
    unittest.main()
