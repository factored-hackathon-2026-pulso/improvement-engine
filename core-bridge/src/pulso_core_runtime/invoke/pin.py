"""Pre-pin checks (step 4): release exists, active, agent/version match, closure digest equals the commitment."""

from __future__ import annotations

import hashlib
from typing import Any, Protocol

from agent_core.domain import EntityKind, canonical_bytes

RELEASE_PIN_UNAVAILABLE = "pulso:release_pin_unavailable"
RELEASE_REVOKED = "pulso:release_revoked"
RELEASE_DRIFT = "pulso:release_drift"


class ReleaseChecker(Protocol):
    def check(self, release_id: str, agent_id: str, agent_version: str, closure_digest: str | None) -> str | None:
        """None when the pin is authorised, else a `pulso:*` error code."""
        ...


def closure_digest(release: Any) -> str:
    """`sha256(JCS(release.entities))` over the pinned release closure."""
    return hashlib.sha256(canonical_bytes(release.entities)).hexdigest()


class RegistryReleaseChecker:
    """Over a registry exposing `release_status(id)` and `release(id)` (the L2 `PinnedRegistryPort`)."""

    def __init__(self, registry: Any) -> None:
        self._registry = registry

    def check(self, release_id: str, agent_id: str, agent_version: str, closure_digest_: str | None) -> str | None:
        try:
            status = self._registry.release_status(release_id)
            release = self._registry.release(release_id)
        except KeyError:
            return RELEASE_PIN_UNAVAILABLE
        status_text = getattr(status, "value", status)
        if status_text == "revoked":
            return RELEASE_REVOKED
        if status_text != "active":
            return RELEASE_PIN_UNAVAILABLE
        version = release.entities.get(EntityKind.agent, {}).get(agent_id)
        if version != agent_version:
            return RELEASE_PIN_UNAVAILABLE
        if closure_digest_ is not None and closure_digest_ != closure_digest(release):
            return RELEASE_PIN_UNAVAILABLE
        return None
