"""Small, dependency-free validator for the JSON Schema keywords used here.

This intentionally fails on unknown validation keywords so a schema edit cannot
silently become a no-op in the local OPBENCH generator.
"""

from __future__ import annotations

import json
import math
import re
from typing import Any


_METADATA = {"$schema", "$id", "title", "description", "default"}
_SUPPORTED = {
    "$ref", "type", "const", "enum", "properties", "required",
    "additionalProperties", "items", "minItems", "maxItems", "uniqueItems",
    "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "pattern",
    "allOf", "oneOf", "anyOf", "if", "then", "else",
}


def validate(instance: Any, schema: dict[str, Any]) -> None:
    """Validate an instance against this repository's supported schema subset."""
    definitions = schema.get("$defs", {})
    errors: list[str] = []
    _validate_schema_tree(schema, "$schema")

    def check(value: Any, rule: dict[str, Any], path: str) -> None:
        unknown = set(rule) - _SUPPORTED - _METADATA - {"$defs"}
        if unknown:
            errors.append(f"{path}: unsupported schema keyword(s): {', '.join(sorted(unknown))}")
            return
        ref = rule.get("$ref")
        if ref is not None:
            prefix = "#/$defs/"
            if not isinstance(ref, str) or not ref.startswith(prefix) or ref[len(prefix):] not in definitions:
                errors.append(f"{path}: unresolved local schema reference")
                return
            check(value, definitions[ref[len(prefix):]], path)
        if "type" in rule:
            expected = rule["type"]
            expected_types = expected if isinstance(expected, list) else [expected]
            if not any(_matches_type(value, kind) for kind in expected_types):
                errors.append(f"{path}: expected {' or '.join(expected_types)}")
                return
        if "const" in rule and not _json_equal(value, rule["const"]):
            errors.append(f"{path}: value does not match const")
        if "enum" in rule and value not in rule["enum"]:
            errors.append(f"{path}: value is outside enum")
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            if isinstance(value, float) and not math.isfinite(value):
                errors.append(f"{path}: non-finite numbers are not valid JSON values")
                return
            for key, predicate in (
                ("minimum", lambda a, b: a >= b),
                ("maximum", lambda a, b: a <= b),
                ("exclusiveMinimum", lambda a, b: a > b),
                ("exclusiveMaximum", lambda a, b: a < b),
            ):
                if key in rule and not predicate(value, rule[key]):
                    errors.append(f"{path}: violates {key}")
        if isinstance(value, str) and "pattern" in rule and re.search(rule["pattern"], value) is None:
            errors.append(f"{path}: value does not match pattern")
        if isinstance(value, list):
            if len(value) < rule.get("minItems", 0):
                errors.append(f"{path}: fewer than minItems")
            if len(value) > rule.get("maxItems", float("inf")):
                errors.append(f"{path}: more than maxItems")
            if rule.get("uniqueItems") and len({_canonical(item) for item in value}) != len(value):
                errors.append(f"{path}: duplicate items")
            if "items" in rule:
                for index, item in enumerate(value):
                    check(item, rule["items"], f"{path}[{index}]")
        if isinstance(value, dict):
            properties = rule.get("properties", {})
            for key in rule.get("required", []):
                if key not in value:
                    errors.append(f"{path}: missing required property {key}")
            for key, item in value.items():
                if key in properties:
                    check(item, properties[key], f"{path}.{key}")
                elif rule.get("additionalProperties") is False:
                    errors.append(f"{path}: unexpected property {key}")
                elif isinstance(rule.get("additionalProperties"), dict):
                    check(item, rule["additionalProperties"], f"{path}.{key}")
        for subrule in rule.get("allOf", []):
            check(value, subrule, path)
        for keyword in ("oneOf", "anyOf"):
            if keyword in rule:
                outcomes = [_matches(value, option, definitions) for option in rule[keyword]]
                valid = sum(outcomes)
                if (keyword == "oneOf" and valid != 1) or (keyword == "anyOf" and valid == 0):
                    errors.append(f"{path}: violates {keyword}")
        if "if" in rule:
            branch = "then" if _matches(value, rule["if"], definitions) else "else"
            if branch in rule:
                check(value, rule[branch], path)

    check(instance, schema, "$" )
    if errors:
        raise ValueError("JSON Schema validation failed: " + "; ".join(errors[:8]))


def _matches(value: Any, schema: dict[str, Any], definitions: dict[str, Any]) -> bool:
    # Isolated branch check used by oneOf/anyOf/if. It shares the same validator.
    wrapper = {"$defs": definitions, **schema}
    try:
        validate(value, wrapper)
        return True
    except ValueError:
        return False


def _matches_type(value: Any, kind: str) -> bool:
    return {
        "object": lambda: isinstance(value, dict),
        "array": lambda: isinstance(value, list),
        "string": lambda: isinstance(value, str),
        "integer": lambda: isinstance(value, int) and not isinstance(value, bool) or isinstance(value, float) and math.isfinite(value) and value.is_integer(),
        "number": lambda: isinstance(value, (int, float)) and not isinstance(value, bool) and (not isinstance(value, float) or math.isfinite(value)),
        "boolean": lambda: isinstance(value, bool),
        "null": lambda: value is None,
    }.get(kind, lambda: False)()


def _json_equal(left: Any, right: Any) -> bool:
    """JSON Schema equality distinguishes booleans from numeric 0/1."""
    if isinstance(left, bool) or isinstance(right, bool):
        return isinstance(left, bool) and isinstance(right, bool) and left is right
    if isinstance(left, (int, float)) and isinstance(right, (int, float)):
        return not isinstance(left, bool) and not isinstance(right, bool) and left == right
    return type(left) is type(right) and left == right


def _validate_schema_tree(schema: Any, path: str) -> None:
    """Eagerly reject unsupported keywords, even in optional schema branches."""
    if isinstance(schema, list):
        for index, item in enumerate(schema):
            _validate_schema_tree(item, f"{path}[{index}]")
        return
    if not isinstance(schema, dict):
        return
    unknown = set(schema) - _SUPPORTED - _METADATA - {"$defs"}
    if unknown:
        raise ValueError(f"unsupported JSON Schema keyword(s) at {path}: {', '.join(sorted(unknown))}")
    for key, value in schema.items():
        if key in {"$defs", "properties"} and isinstance(value, dict):
            for name, child in value.items():
                _validate_schema_tree(child, f"{path}.{key}.{name}")
        elif key in {"allOf", "oneOf", "anyOf", "items", "additionalProperties", "if", "then", "else"}:
            _validate_schema_tree(value, f"{path}.{key}")


def _canonical(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
