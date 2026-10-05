"""Story correlation id (O11Y1): ONE deterministic W3C trace id per story, recomputable by every component.

    trace_id = sha256("pulso.story.v1\\n" + finding_key + "\\n" + run_id)[:32]   (finding_key may be empty)

The engine, and callers of llm-gateway and agent-core, derive the same id and send `traceparent` so their spans join
the bridge's trace. A 16-hex parent span id per stage comes from `span_id(trace_id, stage, attempt)`.
"""
from __future__ import annotations

import hashlib
import re

_TP = re.compile(r"^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$")


def story_trace_id(finding_key: str | None, run_id: str) -> str:
    return hashlib.sha256(f"pulso.story.v1\n{finding_key or ''}\n{run_id}".encode()).hexdigest()[:32]


def span_id(trace_id: str, *parts: str) -> str:
    return hashlib.sha256("|".join([trace_id, *parts]).encode()).hexdigest()[:16]


def build_traceparent(trace_id: str, parent_span_id: str, sampled: bool = True) -> str:
    if not re.fullmatch(r"[0-9a-f]{32}", trace_id) or not re.fullmatch(r"[0-9a-f]{16}", parent_span_id):
        raise ValueError("invalid trace or span id")
    if set(trace_id) == {"0"} or set(parent_span_id) == {"0"}:
        raise ValueError("all-zero id")
    return f"00-{trace_id}-{parent_span_id}-{'01' if sampled else '00'}"


def parse_traceparent(header: str) -> dict:
    m = _TP.match((header or "").strip().lower())
    if not m or set(m[1]) == {"0"} or set(m[2]) == {"0"}:
        raise ValueError("invalid traceparent")
    return {"trace_id": m[1], "parent_span_id": m[2], "sampled": bool(int(m[3], 16) & 1)}
