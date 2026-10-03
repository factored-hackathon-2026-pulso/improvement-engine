"""Exporter service-JWT minting (A02/A03): iss=core-bridge, Ed25519 (EdDSA), typ=JWT, one keypair+kid per audience,
singular `scope`, `sub` = registered exporter binding, TTL <= 5 min, a fresh `jti` on every call."""

from __future__ import annotations

import time
import uuid
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from ..internal.auth import sign_service_jwt

ISSUER = "core-bridge"
# route class -> (audience, scope, purpose)
ROUTES: dict[str, tuple[str, str, str | None]] = {
    "observations": ("control-api", "observations", "platform_observations"),
    "cursor": ("control-api", "observations", None),
    "artifacts": ("lab-broker", "artifact_write", "artifact_upload"),
}


@dataclass(frozen=True)
class AudienceKey:
    kid: str
    private_key: Any  # cryptography Ed25519PrivateKey


class ExporterTokenSigner:
    def __init__(self, keys: dict[str, AudienceKey], *, binding_ref: str, tenant_id: str, ttl_seconds: int = 60,
                 now: Callable[[], float] = time.time) -> None:
        if not 0 < ttl_seconds <= 300:
            raise ValueError("ttl must be within 1..300 s")
        self._keys, self._binding, self._tenant, self._ttl, self._now = keys, binding_ref, tenant_id, ttl_seconds, now

    def token_for(self, route: str) -> str:
        aud, scope, purpose = ROUTES[route]
        key = self._keys[aud]
        now = int(self._now())
        claims: dict[str, Any] = {"iss": ISSUER, "aud": aud, "sub": self._binding, "tenant_id": self._tenant,
                                  "scope": scope, "iat": now, "exp": now + self._ttl, "jti": uuid.uuid4().hex}
        if purpose:
            claims["purpose"] = purpose
        return sign_service_jwt(key.private_key, kid=key.kid, claims=claims)
