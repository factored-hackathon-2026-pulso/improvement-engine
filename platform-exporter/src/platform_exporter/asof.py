"""As-of case reconstruction from events only (spec 32.2 item 3): the mutable `cases` row is never read for state.

An event is visible at `cutoff` only if BOTH `event_time <= cutoff` and `ingested_at <= cutoff` (available_at), so a
case closed after the cutoff is open and has no close_reason in the extract."""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from .source import RawEvent


@dataclass
class CaseState:
    case_id: str
    status: str | None = None
    assigned_to: str | None = None
    close_reason: str | None = None
    opened_at: datetime | None = None
    closed_at: datetime | None = None
    last_sequence: int = 0


def _first(payload: Any, *keys: str) -> Any:
    if isinstance(payload, dict):
        for k in keys:
            if payload.get(k) is not None:
                return payload[k]
    return None


def reconstruct_cases(events: Iterable[RawEvent], cutoff: datetime) -> dict[str, CaseState]:
    out: dict[str, CaseState] = {}
    for ev in sorted(events, key=lambda e: e.sequence):
        if ev.problem or ev.case_id is None or ev.event_time is None or ev.ingested_at is None:
            continue
        if ev.event_time > cutoff or ev.ingested_at > cutoff:
            continue
        st = out.setdefault(ev.case_id, CaseState(ev.case_id))
        st.last_sequence = ev.sequence
        if ev.event_type == "case.created":
            st.status, st.opened_at = _first(ev.payload, "status") or "open", ev.event_time
        elif ev.event_type == "case.status_changed":
            st.status = _first(ev.payload, "to", "to_status", "status") or st.status
            if st.status != "closed":
                st.close_reason, st.closed_at = None, None
        elif ev.event_type == "case.assigned":
            st.assigned_to = _first(ev.payload, "analyst_id", "staff_id", "to") or st.assigned_to
        elif ev.event_type == "case.closed":
            st.status, st.closed_at = "closed", ev.event_time
            st.close_reason = _first(ev.payload, "close_reason", "reason")
    return out
