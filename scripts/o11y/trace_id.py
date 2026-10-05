"""Story correlation id (O11Y1): ONE deterministic W3C trace id per story, recomputable by every component.

    trace_id = sha256("pulso.story.v1\\n" + finding_key + "\\n" + run_id)[:32]   (finding_key may be empty)

The engine, and callers of llm-gateway and agent-core, derive the same id and send `traceparent` so their spans join
the bridge's trace. A 16-hex parent span id per stage comes from `span_id(trace_id, stage, attempt)`.

Derivation (a Rust implementation must produce identical bytes; test vectors in fixtures/traceparent_vectors.json):
    trace_id        = hex(sha256(utf8("pulso.story.v1" + "\n" + finding_key_or_empty + "\n" + run_id)))[0:32]
    span_id(t, *p)  = hex(sha256(utf8("|".join([t, *p]))))[0:16]
    root span       = span_id(t, "story")
    stage span      = span_id(t, "stage", <stage>, <attempt as decimal, default "1">)
    generation span = span_id(t, "gen", <role>, <n as decimal, 1-based per role>)
    traceparent     = "00-" + trace_id + "-" + <parent span id> + "-01"
Lowercase hex, no padding. The engine sends traceparent_for(finding_key, run_id, stage, attempt) on gateway and
agent-core calls so their spans become children of the engine's stage span in the same trace.
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


STAGES = ("sensor", "scout", "verifier", "builder", "recompute", "compile", "deliver", "evaluate", "dossier")


def root_span_id(trace_id: str) -> str:
    return span_id(trace_id, "story")


def stage_span_id(trace_id: str, stage: str, attempt: int = 1) -> str:
    return span_id(trace_id, "stage", stage, str(attempt))


def generation_span_id(trace_id: str, role: str, n: int = 1) -> str:
    return span_id(trace_id, "gen", role, str(n))


def traceparent_for(finding_key: str | None, run_id: str, stage: str | None = None, attempt: int = 1) -> str:
    """The header the engine sends on gateway / agent-core calls. Parent = the stage span, or the root span without a stage."""
    t = story_trace_id(finding_key, run_id)
    return build_traceparent(t, root_span_id(t) if stage is None else stage_span_id(t, stage, attempt))
