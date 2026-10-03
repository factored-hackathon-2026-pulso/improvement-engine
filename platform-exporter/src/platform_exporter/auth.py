"""Service-JWT minting for the exporter -> Pulso routes (A02/A03, ADR 0009; same shape as the core-exporter signer).

Ed25519 (EdDSA, `typ=JWT`), one keypair + `kid` per audience, `iss=core-bridge`, singular `scope`, `sub` = the
registered exporter source binding, `tenant_id`, `iat`/`exp` (TTL <= 5 min, default 60 s) and a fresh `jti` on every
call. The caller asks for a token on every HTTP attempt, so a retry never reuses a `jti`. The seed lives only in the
0400 key file written by the container entrypoint; it is read once into a key object and never logged or persisted."""

from __future__ import annotations

import base64
import binascii
import json
import time
import uuid
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

ISSUER = "core-bridge"
MARKER = "pulso:runtime_config_invalid"
# route class -> (audience, scope, purpose)
ROUTES: dict[str, tuple[str, str, str | None]] = {
    "observations": ("control-api", "observations", "platform_observations"),
    "cursor": ("control-api", "observations", None),
    "artifacts": ("lab-broker", "artifact_write", "artifact_upload"),
}


class RuntimeConfigInvalid(Exception):
    """Fail-closed configuration error. The message names variables/audiences only, never a value."""

    def __init__(self, detail: str) -> None:
        super().__init__(f"{MARKER}: {detail}")


@dataclass(frozen=True)
class AudienceKey:
    kid: str
    private_key: Ed25519PrivateKey

    def __repr__(self) -> str:  # never expose key material through repr/logging
        return f"AudienceKey(kid={self.kid!r})"


def _b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def load_seed_file(path: str, *, var: str) -> Ed25519PrivateKey:
    """Read a base64url 32-byte Ed25519 seed file. Any problem -> RuntimeConfigInvalid naming `var` only."""
    try:
        text = Path(path).read_text(encoding="ascii").strip()
        seed = base64.urlsafe_b64decode(text + "=" * (-len(text) % 4)) if text else b""
        if len(seed) != 32:
            raise ValueError
        return Ed25519PrivateKey.from_private_bytes(seed)
    except (OSError, ValueError, binascii.Error, UnicodeError):
        raise RuntimeConfigInvalid(f"{var} is missing or malformed (a readable key file with a base64url 32-byte "
                                   "Ed25519 seed is required)") from None


class ServiceTokenSigner:
    def __init__(self, keys: dict[str, AudienceKey], *, binding_ref: str, tenant_id: str, ttl_seconds: int = 60,
                 now: Callable[[], float] = time.time) -> None:
        if not 0 < ttl_seconds <= 300:
            raise ValueError("ttl must be within 1..300 s")
        self._keys, self._binding, self._tenant, self._ttl, self._now = keys, binding_ref, tenant_id, ttl_seconds, now

    def token_for(self, route: str) -> str:
        aud, scope, purpose = ROUTES[route]
        key = self._keys.get(aud)
        if key is None:
            raise RuntimeConfigInvalid(f"no signing key configured for audience {aud} (route {route}); the key of "
                                       "another audience is never reused")
        now = int(self._now())
        claims: dict[str, Any] = {"iss": ISSUER, "aud": aud, "sub": self._binding, "tenant_id": self._tenant,
                                  "scope": scope, "iat": now, "exp": now + self._ttl, "jti": uuid.uuid4().hex}
        if purpose:
            claims["purpose"] = purpose
        head = _b64url(json.dumps({"alg": "EdDSA", "kid": key.kid, "typ": "JWT"}, separators=(",", ":")).encode())
        body = _b64url(json.dumps(claims, separators=(",", ":")).encode())
        return f"{head}.{body}.{_b64url(key.private_key.sign(f'{head}.{body}'.encode()))}"
