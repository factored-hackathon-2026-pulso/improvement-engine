"""Versioned event catalog, payload treatment and time helpers (spec 32.2 items 4-5)."""

from __future__ import annotations

import hashlib
import re
from collections.abc import Mapping
from datetime import UTC, datetime
from typing import Any

import rfc8785

CATALOG_VERSION = "platform_live.events/1"
SOURCE_NAMESPACE = "platform_live"

# platform-contract revision implemented here (a conformance test compares it with the contract package): exporter
# metadata is `source_event.kind = "exporter_finding"`; ExporterConfig.legacy_prefix=True keeps the 1.0.0 `exporter.`
# event_type prefix shape.
CONTRACT_REVISION = "1.3.0"
FINDING_SEVERITY = {"bad_row": "error", "late_event": "info", "capability_profile": "info",
                    "dimension_snapshot": "info"}  # every other finding code defaults to "warning"

# Mirror of platform-contract/event-catalog.json (a conformance test fails on drift). Closed catalog admitted by
# default; `auth.*` is default-deny except the four allow-listed telemetry types.
KNOWN_EVENT_TYPES = frozenset({
    "case.opened", "case.queued", "case.assigned", "case.status_changed", "case.read", "case.first_responded",
    "case.closed", "case.viewed", "turn.created", "staff.availability_changed",
    "auth.login_failed", "auth.account_locked", "auth.session_started", "auth.session_ended",
    # 1.2.0 (platform eeb73a8): ids, enums and counters only; free-text keys are in FREE_TEXT_PAYLOAD_KEYS.
    "case.priority_changed", "case.rated", "case.assistant_started", "case.assistant_released",
    "assistant.session_started", "assistant.input_queued", "assistant.turn_answered", "assistant.step_up_verified",
    "assistant.step_up_rejected", "assistant.ended", "copilot.query_asked", "copilot.answered",
    "builder.proposal_created", "builder.proposal_tracked", "builder.draft_saved", "builder.proposal_validated",
    "builder.proposal_frozen", "builder.proposal_reopened", "builder.proposal_evaluated",
    "builder.proposal_approved", "builder.proposal_rejected", "builder.proposal_published", "builder.alias_promoted",
    "builder.release_revoked", "builder.question_asked", "builder.answered",
    "escalation.opened", "escalation.withdrawn", "escalation.answered", "escalation.taken", "escalation.reassigned",
    "escalation.closed", "escalation.acknowledged",
    "call.started", "call.answered", "call.held", "call.resumed", "call.mute_changed", "call.ended",
    # 1.3.0 (platform 5261ecf): copilot suggestions, tool feedback, case type, AI maturity, AI switch. Ids, enums,
    # counters and flags only (no free-text key); `subject` of `suggestion_decided` is a closed enum, see below.
    "copilot.suggestion_requested", "copilot.suggestion_ready", "copilot.suggestion_none", "copilot.suggestion_failed",
    "copilot.suggestion_decided", "copilot.tool_used", "case.type_changed",
    "ai.stage_advanced", "ai.stage_moved_back", "ai.agent_ready", "ai.agent_activated", "platform.ai_toggled",
})
# Payload keys that carry free text for a given type (mirror of `free_text_keys` in the contract catalog, drift-tested).
# Dropped for that type even when the key name would not trip the generic redaction tokens (`motive`, `reason`,
# `answer`): the platform puts a customer's/analyst's words there.
FREE_TEXT_PAYLOAD_KEYS = {
    "case.rated": frozenset({"comment"}),
    "assistant.input_queued": frozenset({"answer"}),
    "escalation.opened": frozenset({"motive"}),
    "escalation.answered": frozenset({"note"}),
    "call.started": frozenset({"reason"}),
    "case.closed": frozenset({"note"}),
    "turn.created": frozenset({"text", "subject", "staff_line"}),
}
# Top-level payload keys with a closed value set (mirror of `payload_enums` in the contract catalog, drift-tested). Such a
# key is exempt from the generic name-based redaction (`subject` is a redacted token) but ONLY while its value is in the
# set; any other value (free text where an enum was promised) is redacted and recorded like any other removed key.
ENUM_PAYLOAD_KEYS = {
    "copilot.suggestion_requested": {"trigger": frozenset({"customer_message", "manual", "handover"})},
    "copilot.suggestion_decided": {"subject": frozenset({"reply", "escalation"}),
                                   "decision": frozenset({"used", "edited", "discarded", "ignored", "accepted"})},
}
# Known security/credential telemetry: never ingested, payload never forwarded (counted as denied_event_type).
DENIED_EVENT_TYPES = frozenset({"auth.password_accepted", "auth.mfa_challenge_issued", "auth.mfa_failed",
                                "customer.session_started"})
# Announced by Product but not admitted yet: quarantined like an unknown type (finding says catalog_status=planned).
PLANNED_PREFIXES = ("staff.", "team.")

# Keys never forwarded, at any depth: message text, names, contact data, credentials, free-text notes. A key is
# redacted when it equals one of REDACTED_KEYS or when ANY of its tokens (snake_case / camelCase / kebab split) is in
# REDACTED_TOKENS, so `customer_email`, `fullName`, `phone_number` or `message_preview` cannot slip through. Keys whose
# last token is an identifier/count marker (`message_id`, `name_count`) are kept: they carry no free text.
REDACTED_KEYS = frozenset({
    "text", "body", "message", "content", "name", "display_name", "email", "phone", "note", "close_note",
    "password", "password_hash", "token", "secret", "code", "code_hash",
})
REDACTED_TOKENS = frozenset({
    "text", "body", "message", "msg", "content", "name", "names", "email", "emails", "mail", "phone", "mobile", "note",
    "notes", "comment", "comments", "subject", "preview", "snippet", "address", "password", "passwd", "token", "secret",
    "hash", "username", "otp", "transcript", "description", "title", "summary",
})
_KEEP_LAST_TOKENS = frozenset({"id", "ids", "ref", "refs", "count", "sequence", "seq", "at", "length", "len"})
_TOKEN_SPLIT = re.compile(r"[^A-Za-z0-9]+|(?<=[a-z0-9])(?=[A-Z])")
_EMAIL_VALUE = re.compile(r"[A-Za-z0-9._%+\-]{1,64}@[A-Za-z0-9.\-]{1,255}\.[A-Za-z0-9]{2,63}")  # bounded: no ReDoS
EMAIL_PLACEHOLDER = "[redacted-email]"


def is_redacted_key(key: Any) -> bool:
    k = str(key)
    if k.lower() in REDACTED_KEYS:
        return True
    tokens = [t.lower() for t in _TOKEN_SPLIT.split(k) if t]
    if not tokens or tokens[-1] in _KEEP_LAST_TOKENS:
        return False
    return any(t in REDACTED_TOKENS for t in tokens)


def treat_payload(value: Any, redacted: list[str] | None = None, path: str = "",
                  drop_keys: frozenset[str] = frozenset(),
                  enums: Mapping[str, frozenset[str]] | None = None) -> Any:
    """Copy of `value` without redacted keys (and without `drop_keys`, the free-text keys declared for the event
    type); the removed key paths are appended to `redacted`. Email-looking string values under any other key are
    masked as well (and recorded). `enums` (top level only) maps a key to its closed value set: such a key is kept when its
    value is in the set and redacted otherwise, whatever its name."""
    red = redacted if redacted is not None else []
    if isinstance(value, dict):
        out: dict[str, Any] = {}
        for k, v in value.items():
            p = f"{path}.{k}" if path else str(k)
            if enums and not path and k in enums:
                if isinstance(v, str) and v in enums[k]:
                    out[k] = v
                else:
                    red.append(p)
            elif is_redacted_key(k) or k in drop_keys:
                red.append(p)
            else:
                out[k] = treat_payload(v, red, p, drop_keys)
        return out
    if isinstance(value, list):
        return [treat_payload(v, red, path, drop_keys) for v in value]
    if isinstance(value, str) and "@" in value and _EMAIL_VALUE.search(value):
        red.append(path or "$")
        return _EMAIL_VALUE.sub(EMAIL_PLACEHOLDER, value)
    return value


def parse_ts(value: Any) -> datetime:
    """datetime | ISO-8601 text | epoch seconds/milliseconds -> aware UTC datetime. ValueError when unusable."""
    if isinstance(value, datetime):
        dt = value
    elif isinstance(value, int | float) and not isinstance(value, bool):
        v = float(value)
        dt = datetime.fromtimestamp(v / 1000.0 if v > 1e11 else v, UTC)
    elif isinstance(value, str) and value.strip():
        text = value.strip()
        dt = datetime.fromisoformat(text[:-1] + "+00:00" if text.endswith(("Z", "z")) else text)
    else:
        raise ValueError(f"unusable timestamp: {value!r}")
    return dt.replace(tzinfo=UTC) if dt.tzinfo is None else dt.astimezone(UTC)


def fmt_ts(dt: datetime) -> str:
    base = dt.astimezone(UTC)
    return base.strftime("%Y-%m-%dT%H:%M:%S") + (f".{base.microsecond:06d}" if base.microsecond else "") + "Z"


def jcs_digest(value: Any) -> str:
    return hashlib.sha256(rfc8785.dumps(value)).hexdigest()
