"""Pulso service-JWT verification (A02/A03): `typ=JWT`, verifier-fixed EdDSA, `kid` lookup, per-route audience
and purpose, `exp <= 5 min`, receiver-owned `jti` replay table. Distinct from the Core `principal+jws`."""

from __future__ import annotations

import base64
import json
import math
import threading
from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Protocol

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

MAX_TTL_S = 300
SERVICE_KEYS_SCHEMA = "keys: {kid: {iss, aud, key(b64url ed25519 public, 32 bytes)}}"


class AuthError(Exception):
    """`reason` is a closed code, safe to return; never echoes token content."""

    def __init__(self, reason: str, *, status: int = 401) -> None:
        super().__init__(reason)
        self.reason, self.status = reason, status


class JtiStore(Protocol):
    def consume(self, iss: str, jti: str, exp: datetime) -> bool:
        """Atomically record `(iss, jti)`; False when already seen (replay)."""
        ...


class InMemoryJtiStore:
    def __init__(self) -> None:
        self._seen: dict[tuple[str, str], datetime] = {}
        self._lock = threading.Lock()

    def consume(self, iss: str, jti: str, exp: datetime) -> bool:
        now = datetime.now(UTC)
        with self._lock:
            for key in [k for k, e in self._seen.items() if e < now]:
                del self._seen[key]
            if (iss, jti) in self._seen:
                return False
            self._seen[(iss, jti)] = exp
            return True


def b64url_decode(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def b64url_encode(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


@dataclass(frozen=True)
class ServiceKey:
    iss: str
    aud: str
    key: Ed25519PublicKey


def load_service_keys(path: Path) -> dict[str, ServiceKey]:
    """File: `{"keys": {kid: {"iss", "aud", "key"}}}`. Raises ValueError naming the kid, never the value."""
    try:
        data = json.loads(path.read_bytes())
        raw = data["keys"]
        out: dict[str, ServiceKey] = {}
        for kid, entry in raw.items():
            material = b64url_decode(entry["key"])
            if len(material) != 32:
                raise ValueError(f"service key {kid}: not 32 bytes")
            out[kid] = ServiceKey(entry["iss"], entry["aud"], Ed25519PublicKey.from_public_bytes(material))
    except (OSError, KeyError, TypeError, AttributeError, json.JSONDecodeError) as exc:
        raise ValueError(f"service keys unreadable ({path.name}): {type(exc).__name__}") from None
    if not out:
        raise ValueError("service keys: empty")
    return out


@dataclass(frozen=True)
class Claims:
    iss: str
    aud: str
    sub: str
    purpose: str | None
    tenant_id: str | None
    scope: str | None
    jti: str
    raw: dict[str, Any]


class ServiceJwtVerifier:
    def __init__(self, keys: dict[str, ServiceKey], jti: JtiStore,
                 now: Callable[[], datetime] = lambda: datetime.now(UTC)) -> None:
        self._keys, self._jti, self._now = keys, jti, now

    def verify(self, token: str, *, audience: str, purposes: frozenset[str], require_tenant: bool = True,
               sub_prefix: str | None = None) -> Claims:
        """`sub_prefix`: when set, `sub` must be `<prefix><non-empty id>` (class (i): `worker:<id>`, annex D/A03)."""
        parts = token.split(".")
        if len(parts) != 3:
            raise AuthError("malformed")
        try:
            header = json.loads(b64url_decode(parts[0]))
            payload = json.loads(b64url_decode(parts[1]))
            signature = b64url_decode(parts[2])
        except (ValueError, UnicodeDecodeError):
            raise AuthError("malformed") from None
        if not isinstance(header, dict) or not isinstance(payload, dict):
            raise AuthError("malformed")
        # Verifier-fixed algorithm and type; header is exactly {alg, kid, typ}.
        if header.get("alg") != "EdDSA" or header.get("typ") != "JWT" or set(header) != {"alg", "kid", "typ"}:
            raise AuthError("bad_header")
        entry = self._keys.get(header["kid"]) if isinstance(header["kid"], str) else None
        if entry is None:
            raise AuthError("unknown_kid")
        try:
            entry.key.verify(signature, f"{parts[0]}.{parts[1]}".encode())
        except InvalidSignature:
            raise AuthError("bad_signature") from None
        iss, aud, jti, exp = payload.get("iss"), payload.get("aud"), payload.get("jti"), payload.get("exp")
        if iss != entry.iss or aud != entry.aud:
            raise AuthError("key_binding")  # kid is bound to one (iss, aud)
        if aud != audience:
            raise AuthError("wrong_audience")
        if not isinstance(exp, int | float) or not isinstance(jti, str) or not jti or not isinstance(iss, str):
            raise AuthError("missing_claims")
        iat = payload.get("iat")
        # JSON `NaN`/`Infinity` and booleans must never reach the comparisons (NaN defeats every `>`/`<=`).
        for stamp in (exp, iat):
            if isinstance(stamp, bool) or not isinstance(stamp, int | float) or not math.isfinite(stamp):
                raise AuthError("missing_claims")
        now = self._now().timestamp()
        if exp <= now:
            raise AuthError("expired")
        if exp - iat > MAX_TTL_S or exp - now > MAX_TTL_S + 30:
            raise AuthError("ttl_too_long")
        sub, tenant = payload.get("sub"), payload.get("tenant_id")
        if require_tenant and (not isinstance(tenant, str) or not tenant):
            raise AuthError("tenant_required", status=403)  # every tenant route is scoped by a signed tenant claim
        purpose = payload.get("purpose")
        if purposes and purpose not in purposes:
            raise AuthError("purpose_denied", status=403)
        if not isinstance(sub, str) or not sub:
            raise AuthError("missing_claims")
        if sub_prefix is not None and not (sub.startswith(sub_prefix) and len(sub) > len(sub_prefix)):
            raise AuthError("sub_not_worker", status=403)
        # Consume last, so a rejected token never burns its jti.
        if not self._jti.consume(iss, jti, datetime.fromtimestamp(exp, UTC)):
            raise AuthError("jti_replayed")
        return Claims(iss=iss, aud=aud, sub=sub, purpose=purpose if isinstance(purpose, str) else None,
                      tenant_id=tenant if isinstance(tenant, str) else None,
                      scope=payload.get("scope") if isinstance(payload.get("scope"), str) else None,
                      jti=jti, raw=payload)


def sign_service_jwt(private_key: Any, *, kid: str, claims: dict[str, Any]) -> str:
    """Test/dev helper (Ed25519 private key); production signers live on the sender side."""
    header = b64url_encode(json.dumps({"alg": "EdDSA", "kid": kid, "typ": "JWT"}, separators=(",", ":")).encode())
    body = b64url_encode(json.dumps(claims, separators=(",", ":")).encode())
    return f"{header}.{body}.{b64url_encode(private_key.sign(f'{header}.{body}'.encode()))}"
