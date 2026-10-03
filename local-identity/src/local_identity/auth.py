"""Service-JWT verification for control-api -> human-issuer: `typ=JWT`, verifier-fixed EdDSA, `kid` bound to one
(iss, aud), per-route purpose, TTL <= 60 s, skew boundaries (reject when now >= exp + skew or iat > now + skew),
tenant claim, receiver-owned `(iss, jti)` consume. A Core `principal+jws` is never a valid bearer here."""

from __future__ import annotations

import json
import math
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from cryptography.exceptions import InvalidSignature

from local_identity import CALLER_AUDIENCE
from local_identity.config import ServiceKey
from local_identity.keys import b64url_decode
from local_identity.replay import ReplayStore

MAX_TTL_S = 60


class AuthError(Exception):
    """`reason` is a closed code, safe to return; never echoes token content."""

    def __init__(self, reason: str, *, status: int = 401) -> None:
        super().__init__(reason)
        self.reason, self.status = reason, status


@dataclass(frozen=True)
class Claims:
    iss: str
    sub: str
    tenant_id: str
    purpose: str
    jti: str


def _number(value: Any) -> bool:
    if isinstance(value, bool) or not isinstance(value, int | float):
        return False
    try:
        return math.isfinite(value)
    except OverflowError:  # ints beyond float range
        return False


class ServiceJwtVerifier:
    def __init__(
        self, keys: Mapping[str, ServiceKey], replay: ReplayStore, now: Callable[[], datetime], skew_s: int
    ) -> None:
        self._keys, self._replay, self._now, self._skew = keys, replay, now, skew_s

    def verify(self, token: str, *, purpose: str) -> Claims:
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
        if header.get("alg") != "EdDSA" or header.get("typ") != "JWT" or set(header) != {"alg", "kid", "typ"}:
            raise AuthError("bad_header")
        entry = self._keys.get(header["kid"]) if isinstance(header["kid"], str) else None
        if entry is None:
            raise AuthError("unknown_kid")
        try:
            entry.key.verify(signature, f"{parts[0]}.{parts[1]}".encode("ascii"))
        except (InvalidSignature, UnicodeEncodeError):
            raise AuthError("bad_signature") from None
        iss, aud, jti, sub = payload.get("iss"), payload.get("aud"), payload.get("jti"), payload.get("sub")
        if iss != entry.iss or aud != entry.aud:
            raise AuthError("key_binding")
        if aud != CALLER_AUDIENCE:
            raise AuthError("wrong_audience")
        exp_raw, iat_raw = payload.get("exp"), payload.get("iat")
        if (
            not _number(exp_raw)
            or not _number(iat_raw)
            or not isinstance(jti, str)
            or not jti
            or not isinstance(sub, str)
            or not sub
            or not isinstance(iss, str)
        ):
            raise AuthError("missing_claims")
        try:
            exp, iat = float(exp_raw), float(iat_raw)  # type: ignore[arg-type]
        except OverflowError:
            raise AuthError("missing_claims") from None
        now = self._now().timestamp()
        if now >= exp + self._skew:
            raise AuthError("expired")
        if iat > now + self._skew:
            raise AuthError("not_yet_valid")
        if exp - iat > MAX_TTL_S or exp - now > MAX_TTL_S + self._skew:
            raise AuthError("ttl_too_long")
        tenant = payload.get("tenant_id")
        if not isinstance(tenant, str) or not tenant:
            raise AuthError("tenant_required", status=403)
        if payload.get("purpose") != purpose:
            raise AuthError("purpose_denied", status=403)
        # Consume last, so a rejected token never burns its jti; retained through exp + skew.
        if not self._replay.consume_jti(iss, jti, retain_until=int(exp) + self._skew + 1, now=int(now)):
            raise AuthError("jti_replayed")
        return Claims(iss=iss, sub=sub, tenant_id=tenant, purpose=purpose, jti=jti)
