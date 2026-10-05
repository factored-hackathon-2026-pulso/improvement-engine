"""Structural parser for privacy-checked six-field aggregate cell NDJSON."""

from __future__ import annotations

import json
import re
from typing import Any

_FIELDS = frozenset({"metric", "dims", "half", "period", "numerator", "denominator"})
_PERIOD = re.compile(r"\d{4}-(?:0[1-9]|1[0-2])\Z")

def parse_cell_ndjson(text: str) -> list[dict[str, Any]]:
    """Parse only the shared row structure; each source still needs its own metric registry."""
    if not isinstance(text, str):
        raise ValueError("cell table must be text")
    rows: list[dict[str, Any]] = []
    for line_number, line in enumerate(text.splitlines(), 1):
        if not line.strip():
            raise ValueError(f"blank cell row at line {line_number}")
        try:
            row = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ValueError(f"invalid cell JSON at line {line_number}") from exc
        if not isinstance(row, dict) or set(row) != _FIELDS:
            raise ValueError(f"cell row at line {line_number} must have the exact six-field schema")
        if not isinstance(row["metric"], str) or not row["metric"]:
            raise ValueError(f"invalid cell metric at line {line_number}")
        dims = row["dims"]
        if not isinstance(dims, dict) or any(
            not isinstance(key, str) or not key or not isinstance(value, str) or not value
            for key, value in dims.items()
        ):
            raise ValueError(f"invalid cell dimensions at line {line_number}")
        if row["half"] not in {"discovery", "holdout"}:
            raise ValueError(f"invalid cell half at line {line_number}")
        if not isinstance(row["period"], str) or not _PERIOD.fullmatch(row["period"]):
            raise ValueError(f"invalid cell period at line {line_number}")
        numerator, denominator = row["numerator"], row["denominator"]
        if (isinstance(numerator, bool) or not isinstance(numerator, int)
                or isinstance(denominator, bool) or not isinstance(denominator, int)
                or denominator < 10 or numerator < 0 or numerator > denominator
                or numerator not in {0, denominator} and min(numerator, denominator - numerator) < 10):
            raise ValueError(f"cell counts violate k=10 at line {line_number}")
        rows.append(row)
    return rows


def render_cell_ndjson(rows: list[dict[str, Any]]) -> str:
    if not isinstance(rows, list):
        raise ValueError("cell rows must be a list")
    ordered = sorted(rows, key=lambda row: (row["period"], row["half"], row["metric"],
                                            json.dumps(row["dims"], sort_keys=True, separators=(",", ":"))))
    rendered = "".join(
        json.dumps(row, ensure_ascii=True, sort_keys=True, separators=(",", ":")) + "\n"
        for row in ordered
    )
    parse_cell_ndjson(rendered)
    return rendered
