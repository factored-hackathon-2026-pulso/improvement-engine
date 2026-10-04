"""Release events (P2py): candidate additions to the event catalog, NOT admitted in catalog 1.1.0.

`release.published` / `release.rolled_back` describe that an agent release moved an alias. The
payload carries identity only. It must never carry an effect size or a reference to a planted
detection mechanism: effects are authored apart (see platform-sim/platform_live/effects.py).
Under catalog 1.1.0 these types classify as `unknown` (quarantined with a finding) until the
engine admits them (P2R)."""

RELEASE_EVENT_TYPES = ("release.published", "release.rolled_back")
REQUIRED = ("release_id", "agent_id", "alias")
OPTIONAL = ("previous_release_id",)


def validate_release_payload(event_type: str, payload: dict) -> list[str]:
    """Violation messages (empty = valid). Messages name fields, never echo values."""
    if event_type not in RELEASE_EVENT_TYPES:
        return ["not a release event type"]
    out = [f"missing field {k}" for k in REQUIRED if k not in payload]
    out += [f"field not allowed: {k}" for k in payload if k not in REQUIRED + OPTIONAL]
    return out
