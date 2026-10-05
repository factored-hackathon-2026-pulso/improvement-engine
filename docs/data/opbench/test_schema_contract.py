"""The published v2 schema is applied to the actual synthetic catalog output."""

from __future__ import annotations

import copy
import unittest

import test_v2
from generate_v2 import validate_v2_artifacts
from json_schema import validate


class JsonSchemaContractTests(unittest.TestCase):
    def test_validator_rejects_unknown_keywords_instead_of_ignoring_them(self):
        with self.assertRaisesRegex(ValueError, "unsupported JSON Schema keyword"):
            validate({"value": 1}, {"type": "object", "mysteryKeyword": True})

    def test_validator_rejects_unknown_keywords_in_unselected_branches(self):
        schema = {"if": {"const": True}, "then": {"type": "boolean"},
                  "else": {"type": "string", "futureKeyword": True}}
        with self.assertRaisesRegex(ValueError, "futureKeyword"):
            validate(True, schema)

    def test_const_uses_json_types_not_python_bool_integer_equality(self):
        schema = {"type": "object", "additionalProperties": False,
                  "required": ["flag"], "properties": {"flag": {"const": False}}}
        validate({"flag": False}, schema)
        with self.assertRaisesRegex(ValueError, "const"):
            validate({"flag": 0}, schema)

    def test_non_finite_values_are_rejected_as_invalid_json_numbers(self):
        for value in (float("nan"), float("inf"), float("-inf")):
            with self.subTest(value=value):
                with self.assertRaisesRegex(ValueError, "expected number"):
                    validate(value, {"type": "number"})
        with self.assertRaisesRegex(ValueError, "does not match const"):
            validate(1, {"const": True})

    def test_validator_enforces_references_and_one_of(self):
        schema = {
            "$defs": {"count": {"type": "integer", "minimum": 10}},
            "oneOf": [
                {"type": "object", "additionalProperties": False, "required": ["count"],
                 "properties": {"count": {"$ref": "#/$defs/count"}}},
                {"type": "null"},
            ],
        }
        validate({"count": 10}, schema)
        validate(None, schema)
        with self.assertRaisesRegex(ValueError, "oneOf"):
            validate({"count": 3}, schema)

    def test_generated_catalog_passes_schema_and_unknown_nested_field_fails(self):
        payload, audit = test_v2.generate_v2_from_rows(
            [], [], [], [], [], {},
            {
                "discovery": {"selected": 154, "total": 200},
                "replication": {"selected": 1433, "total": 1800},
                "complaint_ids_matched": 2000,
                "eligible_cases": 2000,
            },
        )
        validate_v2_artifacts(payload, audit)
        tampered = copy.deepcopy(payload)
        tampered["entries"][0]["snapshot"]["unexpected_nested_field"] = "must reject"
        with self.assertRaisesRegex(ValueError, "JSON Schema validation failed"):
            validate_v2_artifacts(tampered, audit)


if __name__ == "__main__":
    unittest.main()
