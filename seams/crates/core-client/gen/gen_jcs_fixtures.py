#!/usr/bin/env python3
"""Generates tests/fixtures/jcs_parity.json: JCS (RFC 8785) canonical forms computed by the Python `rfc8785`
package. Integers and strings only (the Rust canon refuses non-integer numbers on purpose).

Usage (repo root): uv run --with rfc8785 python seams/crates/core-client/gen/gen_jcs_fixtures.py
"""
import json
import pathlib

import rfc8785

OUT = pathlib.Path(__file__).resolve().parents[1] / "tests/fixtures/jcs_parity.json"

CASES = {
    "empty_object": {},
    "empty_array": [],
    "scalars_array": [None, True, False, 0, -1, 9007199254740991, -9007199254740991, "", "a"],
    "key_order_ascii": {"b": 1, "a": 2, "B": 3, "aa": 4, "a1": 5, "": 6},
    "key_order_utf16_vs_utf8": {"￿": 1, "\U00010000": 2, "": 3, "퟿": 4, "z": 5, "\U0001f600": 6},
    "string_escapes": ["\"", "\\", "/", "\b", "\f", "\n", "\r", "\t", "\u0001", "\u001f", "\u007f", "\u0080"],
    "unicode_values": ["é", "中文", "\U0001f600", "  ", "é", "﻿"],
    "unicode_keys_nested": {"é": {"è": [1, {"ê": "ë"}]}, "e": {}},
    "deep_nesting": {"a": {"b": {"c": {"d": [[[[1]]]]}}}},
    "mixed_array_objects": [{"z": 1, "y": [3, 2, 1]}, {"a": None}, []],
    "numeric_strings_and_keys": {"10": "a", "9": "b", "1": "c", "-1": "d", "01": "e"},
    "int_limits": {"max": 9007199254740991, "min": -9007199254740991, "zero": 0, "neg": -10},
    "request_like": {"schema_version": "1", "tenant_id": "t1", "agent_id": "atencion", "base_release_id": None,
                     "changes": [{"kind": "prompt", "content": {"id": "p", "version": "1.0.0", "text": "Hola ¿qué tal?"}, "docs": {}}]},
}


def main() -> None:
    out = []
    for name, value in CASES.items():
        out.append({"name": name, "input": json.dumps(value, ensure_ascii=True), "canonical": rfc8785.dumps(value).decode("utf-8")})
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes((json.dumps({"generated_by": "gen/gen_jcs_fixtures.py", "rfc8785": getattr(rfc8785, "__version__", "unknown"), "cases": out}, indent=1, ensure_ascii=True) + "\n").encode("ascii"))
    print(f"wrote {len(out)} cases to {OUT}")


if __name__ == "__main__":
    main()
