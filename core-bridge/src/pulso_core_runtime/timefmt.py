"""Annex D.1 timestamp format: UTC RFC3339 with a literal `Z` (no offsets, no naive values)."""

from __future__ import annotations

import re
from datetime import datetime

Z_TIMESTAMP_RE = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,9})?Z")


def parse_z_timestamp(value: str) -> datetime:
    if not Z_TIMESTAMP_RE.fullmatch(value):
        raise ValueError("timestamp must be UTC RFC3339 with a Z suffix")
    return datetime.fromisoformat(value[:-1] + "+00:00")
