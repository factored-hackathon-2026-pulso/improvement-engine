"""Privacy-gated Agent Core outcome and tool-use run aggregates."""

from __future__ import annotations

import json
import os
import re
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping


PROTOCOL = "agent-run-aggregates.v3"
DEFAULT_K = 10
_SPLIT_DOMAIN = b"pulso-agent-run-split-v1\0"

# This is a versioned, finite allow-list of source registry IDs to safe labels.
# Never echo registry IDs; IDs absent from this map are intentionally coarsened.
_AGENT_ID_LABELS = {"pulso-builder": "builder"}
_LOCALES = frozenset({"es", "pt"})
_AGENT_ID = re.compile(r"[a-z0-9][a-z0-9_/-]*\Z")
_AGENT_VERSION = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\Z")
_LOCALE_TAG = re.compile(r"[a-z]{2,3}(?:-[A-Za-z0-9]{2,8})*\Z")
_TOOL_STATUSES = frozenset({"ok", "error", "timeout", "denied", "uncertain", "step_up_required"})
_TOOL_ERROR_STATUSES = frozenset({"error", "timeout", "denied"})
_HANDOFF_CLOSED_BY = frozenset({"escalation", "transfer"})
_TOPIC_NOT_OBSERVED = "not_observed"

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


def _events_for_run(events_by_run: object, run_id: str) -> list[Mapping[str, Any]]:
    if not isinstance(events_by_run, Mapping):
        raise ExportContractError("invalid event pages")
    page_trace = events_by_run.get(run_id)
    events = _complete_page_items({"pages": page_trace}, field="event")
    unique: dict[tuple[str, int], Mapping[str, Any]] = {}
    for event in events:
        seq = event.get("seq")
        if event.get("run_id") != run_id:
            raise ExportContractError("event run key does not match page key")
        if isinstance(seq, bool) or not isinstance(seq, int) or seq < 0:
            raise ExportContractError("invalid event sequence")
        key = (run_id, seq)
        existing = unique.get(key)
        if existing is not None and dict(existing) != dict(event):
            raise ExportContractError("conflicting duplicate event sequence")
        unique[key] = event
    result = [unique[key] for key in sorted(unique)]
    for event in result:
        if event.get("type") != "tool_called":
            continue
        payload = event.get("payload")
        status = payload.get("status") if isinstance(payload, Mapping) else None
        attempt = payload.get("attempt", 1) if isinstance(payload, Mapping) else None
        if not isinstance(status, str) or status not in _TOOL_STATUSES:
            raise ExportContractError("unknown tool status")
        if isinstance(attempt, bool) or not isinstance(attempt, int) or attempt < 1:
            raise ExportContractError("invalid tool attempt")
    return result


def _terminal_event(events: list[Mapping[str, Any]]) -> tuple[str, datetime, str]:
    closed = [event for event in events if event.get("type") == "run_closed"]
    if len(closed) != 1:
        raise ExportContractError("expected one terminal event")
    payload = closed[0].get("payload")
    outcome = payload.get("outcome") if isinstance(payload, Mapping) else None
    if not isinstance(outcome, str) or outcome_group(outcome) is None:
        raise ExportContractError("invalid terminal event outcome")
    closed_at = _utc_instant(closed[0].get("ts"), field="terminal event time")
    closed_by = payload.get("closed_by") if isinstance(payload, Mapping) else None
    if not isinstance(closed_by, str) or closed_by not in {
        "flow", "abandonment", "escalation", "revocation", "transfer"
    }:
        raise ExportContractError("invalid terminal close reason")
    return outcome, closed_at, closed_by


def _is_handoff(status: str, events: list[Mapping[str, Any]], closed_by: str | None) -> bool:
    """CL-0075 run-level handoff union; each terminal run contributes at most once."""
    return (
        status == "escalated"
        or any(event.get("type") == "run_transferred" for event in events)
        or closed_by in _HANDOFF_CLOSED_BY
    )


def _safe_dimensions(run: Mapping[str, Any]) -> dict[str, str]:
    agent = run.get("agent")
    if not isinstance(agent, Mapping):
        raise ExportContractError("missing or invalid run agent")
    agent_id = agent.get("id")
    agent_version = agent.get("version")
    if (
        not isinstance(agent_id, str) or not _AGENT_ID.fullmatch(agent_id)
        or not isinstance(agent_version, str) or not _AGENT_VERSION.fullmatch(agent_version)
    ):
        raise ExportContractError("invalid run agent reference")
    agent_label = _AGENT_ID_LABELS.get(agent_id, "other")
    locale = run.get("locale")
    if not isinstance(locale, str) or len(locale) > 35 or not _LOCALE_TAG.fullmatch(locale):
        raise ExportContractError("missing or invalid run locale")
    language = locale.split("-", maxsplit=1)[0]
    locale_label = language if language in _LOCALES else "other"
    return {"agent": agent_label, "locale": locale_label, "topic": _TOPIC_NOT_OBSERVED}


def _selected_runs(export: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    runs = _complete_page_items(export.get("runs"), field="run")
    latest: dict[str, tuple[int, Mapping[str, Any]]] = {}
    for run in runs:
        run_id = run.get("run_id")
        if not isinstance(run_id, str) or not run_id or run_id != run_id.strip():
            raise ExportContractError("invalid run key")
        cursor = run.get("cursor")
        if isinstance(cursor, bool) or not isinstance(cursor, int) or cursor < 0:
            raise ExportContractError("invalid run cursor")
        existing = latest.get(run_id)
        if existing is None or cursor > existing[0]:
            latest[run_id] = (cursor, run)
        elif cursor == existing[0] and dict(run) != dict(existing[1]):
            raise ExportContractError("conflicting run snapshots at same cursor")
    return [run for _, run in sorted(latest.values(), key=lambda item: item[1]["run_id"])]


def _eligible_runs(
    export: Mapping[str, Any],
) -> tuple[dict[tuple[str, str, str], Counter[str]], dict[tuple[str, str, str], Counter[str]]]:
    event_pages = export.get("events")
    if not isinstance(event_pages, Mapping):
        raise ExportContractError("invalid event pages")

    tool_groups: dict[tuple[str, str, str], Counter[str]] = defaultdict(Counter)
    terminal: dict[tuple[str, str, str], Counter[str]] = defaultdict(Counter)
    for run in _selected_runs(export):
        run_id = run["run_id"]
        status = run.get("status")
        if not isinstance(status, str) or status not in _RUN_STATUSES:
            raise ExportContractError("unknown run status")
        events = _events_for_run(event_pages, run_id)
        created_at = _utc_instant(run.get("created_at"), field="run creation time")
        dimensions = _safe_dimensions(run)
        half = split_for_run(run_id)
        tool_key = (created_at.strftime("%Y-%m"), half, json.dumps(dimensions, sort_keys=True))
        tool_groups[tool_key]["population"] += 1
        tool_events = [
            {**event["payload"], "attempt": event["payload"].get("attempt", 1)}
            for event in events if event.get("type") == "tool_called"
        ]
        if tool_events:
            tool_groups[tool_key]["denominator"] += 1
            tool_groups[tool_key]["tool_error"] += int(
                any(event["status"] in _TOOL_ERROR_STATUSES for event in tool_events)
            )
            tool_groups[tool_key]["tool_retry"] += int(any(event["attempt"] > 1 for event in tool_events))

        if status == "open":
            continue
        outcome = run.get("outcome")
        if not isinstance(outcome, str) or outcome_group(outcome) is None:
            raise ExportContractError("unknown terminal outcome")
        closed_at = _utc_instant(run.get("closed_at"), field="close time")
        event_outcome, event_closed_at, closed_by = _terminal_event(events)
        if event_outcome != outcome:
            raise ExportContractError("run and terminal event disagree")
        if event_closed_at.strftime("%Y-%m") != closed_at.strftime("%Y-%m"):
            raise ExportContractError("run and terminal event close months disagree")
        terminal_key = (closed_at.strftime("%Y-%m"), half, json.dumps(dimensions, sort_keys=True))
        outcome_bucket = outcome_group(outcome)
        terminal[terminal_key][outcome_bucket] += 1
        terminal[terminal_key]["__denominator__"] += 1
        is_handoff = _is_handoff(status, events, closed_by)
        if is_handoff:
            terminal[terminal_key]["__handoff__"] += 1
            terminal[terminal_key]["__handoff_outcome__" + outcome_bucket] += 1
    return tool_groups, terminal


def aggregate_export(export: Mapping[str, Any]) -> dict[str, Any]:
    """Return a safe aggregate report; sub-k outcome vectors are withheld whole."""
    if not isinstance(export, Mapping):
        raise ExportContractError("invalid export")
    evidence_class = _evidence_class(export.get("_label"))
    tool_groups, terminal = _eligible_runs(export)

    cells: list[dict[str, Any]] = []
    suppressed_any = False
    for (month, half, dims_json), counts in sorted(terminal.items()):
        dimensions = json.loads(dims_json)
        denominator = counts["__denominator__"]
        # Jointly suppress outcome and handoff marginals. Releasing either
        # marginal alone can disclose a sub-k outcome × handoff cell by
        # differencing against the other. The internal 5x2 table, including
        # each complementary cell, must clear k before either marginal ships.
        if any(
            counts[metric] < DEFAULT_K or denominator - counts[metric] < DEFAULT_K
            for metric in _OUTCOME_GROUPS
        ) or any(
            counts["__handoff_outcome__" + metric] < DEFAULT_K
            or counts[metric] - counts["__handoff_outcome__" + metric] < DEFAULT_K
            for metric in _OUTCOME_GROUPS
        ):
            suppressed_any = True
            continue
        for metric in _OUTCOME_GROUPS:
            outcome_count = counts[metric]
            cells.append(
                {
                    "metric": "AG_RUN_OUTCOME_RATE",
                    "dims": {**dimensions, "outcome": metric},
                    "half": half,
                    "period": month,
                    "numerator": outcome_count,
                    "denominator": denominator,
                }
            )
        handoff = counts["__handoff__"]
        cells.append({
            "metric": "AG_RUN_HANDOFF_RATE", "dims": dimensions, "half": half,
            "period": month, "numerator": handoff, "denominator": denominator,
        })

    for (month, half, dims_json), group in sorted(tool_groups.items()):
        denominator = group["denominator"]
        dimensions = json.loads(dims_json)
        # The denominator is itself a selected cohort. Protect the complement
        # (runs with no tool event) against subtraction from the full run count.
        if denominator == 0:
            continue
        if group["population"] - denominator < DEFAULT_K:
            suppressed_any = True
            continue
        for metric, field in (("AG_TOOL_ERROR_RUN_RATE", "tool_error"), ("AG_TOOL_RETRY_RUN_RATE", "tool_retry")):
            numerator = group[field]
            if numerator < DEFAULT_K or denominator - numerator < DEFAULT_K:
                suppressed_any = True
                continue
            cells.append({
                "metric": metric, "dims": dimensions, "half": half, "period": month,
                "numerator": numerator, "denominator": denominator,
            })

    cells.sort(key=lambda row: (row["period"], row["half"], row["metric"], json.dumps(row["dims"], sort_keys=True)))

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
    payload = (json.dumps(report, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
    return write_immutable_bytes(payload, output_path, repo_root=repo_root)


def write_immutable_bytes(payload: bytes, output_path: str | Path, *, repo_root: str | Path) -> Path:
    """Write already-serialized output once outside the checkout."""
    if not isinstance(payload, bytes):
        raise ExportContractError("report payload must be bytes")
    root = Path(repo_root).resolve(strict=True)
    target = Path(output_path).expanduser().resolve(strict=False)
    if target == root or root in target.parents:
        raise ExportContractError("report destination must be outside repository")
    if target.exists() or target.is_symlink():
        raise ExportContractError("refusing to overwrite report")
    if not target.parent.is_dir():
        raise ExportContractError("report parent directory must exist")

    try:
        with target.open("xb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
    except FileExistsError as exc:
        raise ExportContractError("refusing to overwrite report") from exc
    return target
