"""A03 service JWTs: `typ=JWT`, EdDSA, header exactly {alg, kid, typ}, kid bound to one (iss, aud), receiver-owned
`jti` replay set. Signing is the sender side; `Verifier` is what the control-api / lab-broker doubles run."""

from __future__ import annotations

import base64
import json
import threading
import time
from collections.abc import Callable
from typing import Any

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey
from cryptography.hazmat.primitives.serialization import Encoding, NoEncryption, PrivateFormat, PublicFormat


def b64u(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def b64d(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def private_from_seed(seed_b64u: str) -> Ed25519PrivateKey:
    return Ed25519PrivateKey.from_private_bytes(b64d(seed_b64u.strip()))


def seed_of(key: Ed25519PrivateKey) -> str:
    return b64u(key.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption()))


def public_of(key: Ed25519PrivateKey) -> str:
    return b64u(key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw))


def sign(key: Ed25519PrivateKey, kid: str, claims: dict[str, Any]) -> str:
    head = b64u(json.dumps({"alg": "EdDSA", "kid": kid, "typ": "JWT"}, separators=(",", ":")).encode())
    body = b64u(json.dumps(claims, separators=(",", ":")).encode())
    return f"{head}.{body}.{b64u(key.sign(f'{head}.{body}'.encode()))}"


class Denied(Exception):
    def __init__(self, reason: str, status: int = 401) -> None:
        super().__init__(reason)
        self.reason, self.status = reason, status


class KeyRing:
    """{kid: (iss, aud, public key b64url)}."""

    def __init__(self, entries: dict[str, tuple[str, str, str]]) -> None:
        self.entries = {kid: (iss, aud, Ed25519PublicKey.from_public_bytes(b64d(pk)))
                        for kid, (iss, aud, pk) in entries.items()}


class Verifier:
    def __init__(self, ring: KeyRing, now: Callable[[], float] = time.time) -> None:
        self._ring, self._now = ring, now
        self._seen: set[tuple[str, str]] = set()
        self._lock = threading.Lock()
        self.accepted: list[dict[str, Any]] = []  # claims of accepted tokens (no token material)

    def verify(self, token: str, *, aud: str, scope: str | None = None, purpose: str | None = None) -> dict[str, Any]:
        parts = token.split(".")
        if len(parts) != 3:
            raise Denied("malformed")
        try:
            head, claims, sig = json.loads(b64d(parts[0])), json.loads(b64d(parts[1])), b64d(parts[2])
        except ValueError:
            raise Denied("malformed") from None
        if not isinstance(head, dict) or not isinstance(claims, dict):
            raise Denied("malformed")
        if head.get("alg") != "EdDSA" or head.get("typ") != "JWT" or set(head) != {"alg", "kid", "typ"}:
            raise Denied("bad_header")
        entry = self._ring.entries.get(head["kid"]) if isinstance(head["kid"], str) else None
        if entry is None:
            raise Denied("unknown_kid")
        try:
            entry[2].verify(sig, f"{parts[0]}.{parts[1]}".encode())
        except InvalidSignature:
            raise Denied("bad_signature") from None
        if claims.get("iss") != entry[0] or claims.get("aud") != entry[1] or claims.get("aud") != aud:
            raise Denied("wrong_audience")
        exp, jti = claims.get("exp"), claims.get("jti")
        if not isinstance(exp, int | float) or not isinstance(jti, str) or not jti:
            raise Denied("missing_claims")
        now = self._now()
        if exp <= now:
            raise Denied("expired")
        if exp - now > 330:
            raise Denied("ttl_too_long")
        if scope is not None and claims.get("scope") != scope:
            raise Denied("scope_denied", 403)
        if purpose is not None and claims.get("purpose") != purpose:
            raise Denied("purpose_denied", 403)
        tenant = claims.get("tenant_id")
        if not isinstance(tenant, str) or not tenant:
            raise Denied("tenant_required", 403)
        with self._lock:
            if (claims["iss"], jti) in self._seen:
                raise Denied("jti_replayed")
            self._seen.add((claims["iss"], jti))
            self.accepted.append({k: claims.get(k) for k in ("iss", "aud", "scope", "purpose", "tenant_id", "jti",
                                                              "binding_ref", "job_id")})
        return claims
