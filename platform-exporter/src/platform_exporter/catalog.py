"""Versioned event catalog, payload treatment and time helpers (spec 32.2 items 4-5)."""

from __future__ import annotations

import hashlib
from datetime import UTC, datetime
from typing import Any

import rfc8785

CATALOG_VERSION = "platform_live.events/1"
SOURCE_NAMESPACE = "platform_live"

# Mirror of platform-contract/event-catalog.json 1.0.0 (a conformance test fails on drift). Closed catalog admitted by
# default; `auth.*` is default-deny except the four allow-listed telemetry types.
KNOWN_EVENT_TYPES = frozenset({
    "case.opened", "case.queued", "case.assigned", "case.status_changed", "case.read", "case.first_responded",
    "case.closed", "case.viewed", "turn.created", "staff.availability_changed",
    "auth.login_failed", "auth.account_locked", "auth.session_started", "auth.session_ended",
})
# Known security/credential telemetry: never ingested, payload never forwarded (counted as denied_event_type).
DENIED_EVENT_TYPES = frozenset({"auth.password_accepted", "auth.mfa_challenge_issued", "auth.mfa_failed",
                                "customer.session_started"})
# Announced by Product but not admitted yet: quarantined like an unknown type (finding says catalog_status=planned).
PLANNED_PREFIXES = ("staff.", "team.")

# Keys never forwarded, at any depth: message text, names, contact data, credentials, free-text notes.
REDACTED_KEYS = frozenset({
    "text", "body", "message", "content", "name", "display_name", "email", "phone", "note", "close_note",
    "password", "password_hash", "token", "secret", "code", "code_hash",
})


def treat_payload(value: Any, redacted: list[str] | None = None, path: str = "") -> Any:
    """Copy of `value` without redacted keys; the removed key paths are appended to `redacted`."""
    red = redacted if redacted is not None else []
    if isinstance(value, dict):
        out: dict[str, Any] = {}
        for k, v in value.items():
            p = f"{path}.{k}" if path else str(k)
            if str(k).lower() in REDACTED_KEYS:
                red.append(p)
            else:
                out[k] = treat_payload(v, red, p)
        return out
    if isinstance(value, list):
        return [treat_payload(v, red, path) for v in value]
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
