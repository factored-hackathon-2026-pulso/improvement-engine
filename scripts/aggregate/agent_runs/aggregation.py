"""Privacy-gated aggregates for recorded Agent Core terminal run outcomes."""

from __future__ import annotations

import json
import os
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping


PROTOCOL = "agent-run-outcomes.v2"
DEFAULT_K = 10
_SPLIT_DOMAIN = b"pulso-agent-run-split-v1\0"

_OUTCOME_GROUPS: dict[str, frozenset[str]] = {
    "resolved": frozenset({"resolved"}),
    "escalation_or_transfer": frozenset({"escalated", "transferred"}),
    "abstention_or_clarification_exhausted": frozenset(
        {"abstained", "clarify_exhausted"}
    ),
    "failed": frozenset({"failed"}),
    "other_terminal": frozenset({"cancelled", "completed", "abandoned"}),
}
_OUTCOME_TO_GROUP = {
    outcome: group
    for group, outcomes in _OUTCOME_GROUPS.items()
    for outcome in outcomes
}
_RUN_STATUSES = frozenset({"closed", "open", "escalated"})


class ExportContractError(ValueError):
    """Raised when an export cannot be safely interpreted under the v2 contract."""


def _valid_cursor(value: object, *, allow_none: bool = False) -> bool:
    if value is None:
        return allow_none
    return (
        not isinstance(value, bool)
        and (isinstance(value, int) or isinstance(value, str) and bool(value))
    )


def outcome_group(outcome: object) -> str | None:
    """Return the fixed reporting group for a known Agent Core outcome."""
    if not isinstance(outcome, str):
        return None
    return _OUTCOME_TO_GROUP.get(outcome)


def split_for_run(run_id: str) -> str:
    """Assign a private run key to a stable reporting half, without emitting it."""
    if not isinstance(run_id, str) or not run_id or run_id != run_id.strip():
        raise ExportContractError("invalid run key")
    import hashlib

    digest = hashlib.sha256(_SPLIT_DOMAIN + run_id.encode("utf-8")).digest()
    return "discovery" if digest[0] & 1 == 0 else "holdout"


def _evidence_class(label: object) -> str:
    if not isinstance(label, str):
        raise ExportContractError("missing evidence label")
    normalized = label.strip().upper()
    if normalized.startswith("RECORDED"):
        return "recorded"
    if normalized.startswith("SYNTHETIC"):
        return "synthetic"
    if normalized.startswith("LIVE PLATFORM"):
        return "live_platform"
    raise ExportContractError("unsupported evidence label")


def _utc_instant(value: object, *, field: str) -> datetime:
    if not isinstance(value, str) or not value:
        raise ExportContractError(f"invalid {field}")
    try:
        parsed = datetime.fromisoformat(value[:-1] + "+00:00" if value.endswith("Z") else value)
    except ValueError as exc:
        raise ExportContractError(f"invalid {field}") from exc
    if parsed.tzinfo is None or parsed.utcoffset() != timezone.utc.utcoffset(parsed):
        raise ExportContractError(f"{field} must be timezone-aware UTC")
    return parsed.astimezone(timezone.utc)


def _complete_page_items(page_envelope: object, *, field: str) -> list[Mapping[str, Any]]:
    """Flatten a cursor traversal only when its empty terminal response is present."""
    pages = page_envelope.get("pages") if isinstance(page_envelope, Mapping) else None
    if not isinstance(pages, list) or not pages:
        raise ExportContractError(f"missing {field} page traversal")

    expected_after: object = None
    flattened: list[Mapping[str, Any]] = []
    terminal_seen = False
    for index, page in enumerate(pages):
        if (
            not isinstance(page, Mapping)
            or "requested_after" not in page
            or not _valid_cursor(page.get("requested_after"), allow_none=index == 0)
            or type(page.get("requested_after")) is not type(expected_after)
            or page.get("requested_after") != expected_after
            or "next_after" not in page
            or not _valid_cursor(page.get("next_after"), allow_none=not page.get("items"))
            or not isinstance(page.get("items"), list)
        ):
            raise ExportContractError(f"invalid {field} page traversal")
        items = page["items"]
        if not items:
            if index != len(pages) - 1:
                raise ExportContractError(f"{field} traversal continues after terminal page")
            terminal_seen = True
            break

        flattened.extend(item for item in items if isinstance(item, Mapping))
        if len(flattened) != sum(len(p["items"]) for p in pages[:index + 1]):
            raise ExportContractError(f"invalid {field} row")
        next_after = page["next_after"]
        if not _valid_cursor(next_after) or (type(next_after) is type(expected_after) and next_after == expected_after):
            raise ExportContractError(f"{field} cursor did not advance")
        expected_after = next_after

    if not terminal_seen:
        raise ExportContractError(f"missing empty terminal {field} page")
    return flattened


def _terminal_event(events_by_run: object, run_id: str) -> tuple[str, datetime]:
    if not isinstance(events_by_run, Mapping):
        raise ExportContractError("invalid event pages")
    page_trace = events_by_run.get(run_id)
    events = _complete_page_items({"pages": page_trace}, field="event")
    if any(event.get("run_id") != run_id for event in events):
        raise ExportContractError("event run key does not match page key")
    closed = [event for event in events if event.get("type") == "run_closed"]
    if len(closed) != 1:
        raise ExportContractError("expected one terminal event")
    payload = closed[0].get("payload")
    outcome = payload.get("outcome") if isinstance(payload, Mapping) else None
    if not isinstance(outcome, str) or outcome_group(outcome) is None:
        raise ExportContractError("invalid terminal event outcome")
    closed_at = _utc_instant(closed[0].get("ts"), field="terminal event time")
    return outcome, closed_at


def _eligible_runs(export: Mapping[str, Any]) -> list[tuple[str, str, str]]:
    runs = _complete_page_items(export.get("runs"), field="run")
    event_pages = export.get("events")
    if not isinstance(event_pages, Mapping):
        raise ExportContractError("invalid event pages")

    unique: dict[str, tuple[str, datetime]] = {}
    for run in runs:
        if not isinstance(run, Mapping):
            raise ExportContractError("invalid run row")
        status = run.get("status")
        if not isinstance(status, str) or status not in _RUN_STATUSES:
            raise ExportContractError("unknown run status")
        if status == "open":
            continue

        run_id = run.get("run_id")
        outcome = run.get("outcome")
        if not isinstance(run_id, str) or not run_id or run_id != run_id.strip():
            raise ExportContractError("invalid run key")
        if not isinstance(outcome, str) or outcome_group(outcome) is None:
            raise ExportContractError("unknown terminal outcome")
        closed_at = _utc_instant(run.get("closed_at"), field="close time")
        event_outcome, event_closed_at = _terminal_event(event_pages, run_id)
        if event_outcome != outcome:
            raise ExportContractError("run and terminal event disagree")
        if event_closed_at.strftime("%Y-%m") != closed_at.strftime("%Y-%m"):
            raise ExportContractError("run and terminal event close months disagree")

        current = (outcome, closed_at)
        existing = unique.get(run_id)
        if existing is not None and existing != current:
            raise ExportContractError("conflicting duplicate run key")
        unique[run_id] = current

    return [
        (run_id, outcome, closed_at.strftime("%Y-%m"))
        for run_id, (outcome, closed_at) in sorted(unique.items())
    ]


def aggregate_export(export: Mapping[str, Any]) -> dict[str, Any]:
    """Return a safe aggregate report; sub-k outcome vectors are withheld whole."""
    if not isinstance(export, Mapping):
        raise ExportContractError("invalid export")
    evidence_class = _evidence_class(export.get("_label"))
    runs = _eligible_runs(export)

    grouped: dict[tuple[str, str], Counter[str]] = defaultdict(Counter)
    for run_id, outcome, month in runs:
        grouped[(month, split_for_run(run_id))][outcome_group(outcome)] += 1

    cells: list[dict[str, Any]] = []
    suppressed_any = False
    for (month, half), counts in sorted(grouped.items()):
        # These five buckets partition the denominator. Suppress the vector as
        # a unit so one rare category cannot be recovered from the other four.
        if any(counts[group] < DEFAULT_K for group in _OUTCOME_GROUPS):
            suppressed_any = True
            continue
        denominator = sum(counts.values())
        for metric in _OUTCOME_GROUPS:
            numerator = counts[metric]
            if numerator < DEFAULT_K or denominator - numerator < DEFAULT_K:
                # Defensive; the vector-level gate above should already imply it.
                suppressed_any = True
                continue
            cells.append(
                {
                    "metric": metric,
                    "dims": {},
                    "half": half,
                    "period": month,
                    "numerator": numerator,
                    "denominator": denominator,
                }
            )

    return {
        "protocol": PROTOCOL,
        "evidence_class": evidence_class,
        "availability": (
            "published" if cells else "below_privacy_floor" if suppressed_any else "no_eligible_runs"
        ),
        "suppression_status": (
            "partial" if suppressed_any and cells else "all" if suppressed_any else "none"
        ),
        "cells": cells,
    }


def write_immutable_report(
    report: Mapping[str, Any], output_path: str | Path, *, repo_root: str | Path
) -> Path:
    """Write deterministic JSON once, refusing any path inside the checkout."""
    root = Path(repo_root).resolve(strict=True)
    target = Path(output_path).expanduser().resolve(strict=False)
    if target == root or root in target.parents:
        raise ExportContractError("report destination must be outside repository")
    if target.exists() or target.is_symlink():
        raise ExportContractError("refusing to overwrite report")
    if not target.parent.is_dir():
        raise ExportContractError("report parent directory must exist")

    payload = (json.dumps(report, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
    try:
        with target.open("xb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
    except FileExistsError as exc:
        raise ExportContractError("refusing to overwrite report") from exc
    return target
