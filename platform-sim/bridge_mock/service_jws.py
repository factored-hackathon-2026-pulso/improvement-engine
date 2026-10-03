"""Sim-only service JWTs for the bridge mock (plan D.1): compact JWS, `alg=EdDSA`, header `typ=JWT` + `kid`.

The algorithm is fixed by the verifier, never taken from the payload. Keys derive from fixed public labels and exist only
for simulators/tests (CAP-63). These are NOT Core principal JWS (`typ=principal+jws`, see `registry_mock.jws`)."""

from __future__ import annotations

import base64
import hashlib
import json
import uuid
from datetime import datetime, timedelta, timezone
from typing import Any

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey

from registry_mock.jws import SIM_EPOCH

ALG = "EdDSA"
TYP = "JWT"
CONTROL_KID = "sim-control-api-1"
ISSUER = "control-api"
AUDIENCE = "core-bridge"
MAX_TTL_SECONDS = 300
MAX_TOKEN_CHARS = 8192


def b64url_encode(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def b64url_decode(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def private_key(label: str = "control-api") -> Ed25519PrivateKey:
    return Ed25519PrivateKey.from_private_bytes(hashlib.sha256(f"pulso bridge-sim TEST ONLY key: {label}".encode()).digest())


def public_keys() -> dict[str, Ed25519PublicKey]:
    return {CONTROL_KID: private_key().public_key()}


def iso(value: datetime) -> str:
    return value.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%fZ")


def parse_iso(text: str) -> datetime:
    return datetime.fromisoformat(text.replace("Z", "+00:00"))


def _dumps(value: Any) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def sign(header: dict[str, Any], payload: dict[str, Any], key: Ed25519PrivateKey | None = None) -> str:
    head, body = b64url_encode(_dumps(header)), b64url_encode(_dumps(payload))
    return f"{head}.{body}.{b64url_encode((key or private_key()).sign(f'{head}.{body}'.encode('ascii')))}"


def claims(*, purpose: str = "task_invoke", tenant_id: str = "tenant-a", iss: str = ISSUER, aud: str = AUDIENCE,
           ttl: float = 60, now: datetime = SIM_EPOCH, job_id: str = "job-1", sub: str = "worker:w1") -> dict[str, Any]:
    return {"iss": iss, "aud": aud, "sub": sub, "tenant_id": tenant_id, "purpose": purpose, "job_id": job_id,
            "iat": int(now.timestamp()), "exp": int((now + timedelta(seconds=ttl)).timestamp()), "jti": uuid.uuid4().hex}


def issue(*, kid: str = CONTROL_KID, typ: str = TYP, **kw: Any) -> str:
    return sign({"alg": ALG, "kid": kid, "typ": typ}, claims(**kw))


def verify(token: str, now: datetime) -> dict[str, Any]:
    """Raises ValueError on any doubt (fail closed). Returns the claims. Replay (`jti`) is the caller's store."""
    if not isinstance(token, str) or not token or len(token) > MAX_TOKEN_CHARS:
        raise ValueError("credential")
    head, body, sig = token.split(".")
    header = json.loads(b64url_decode(head))
    if not isinstance(header, dict) or set(header) != {"alg", "kid", "typ"}:
        raise ValueError("header")
    if header["alg"] != ALG or header["typ"] != TYP:
        raise ValueError("header")
    key = public_keys().get(header["kid"])
    if key is None:
        raise ValueError("kid")
    key.verify(b64url_decode(sig), f"{head}.{body}".encode("ascii"))
    payload = json.loads(b64url_decode(body))
    if not isinstance(payload, dict):
        raise ValueError("payload")
    for name in ("iss", "aud", "sub", "tenant_id", "purpose", "job_id", "iat", "exp", "jti"):
        if name not in payload:
            raise ValueError(f"claim {name}")
    if payload["iss"] != ISSUER or payload["aud"] != AUDIENCE:
        raise ValueError("iss/aud")
    if not all(isinstance(payload[k], int) and not isinstance(payload[k], bool) for k in ("iat", "exp")):
        raise ValueError("times")
    if payload["exp"] - payload["iat"] > MAX_TTL_SECONDS or payload["exp"] <= int(now.timestamp()):
        raise ValueError("exp")
    return payload


def canonical(value: Any) -> bytes:
    """RFC 8785 subset: sorted keys, compact, UTF-8; non-integer numbers are refused (decimals travel as strings)."""
    def check(v: Any) -> None:
        if isinstance(v, float):
            raise ValueError("non-integer JSON number")
        if isinstance(v, dict):
            for x in v.values():
                check(x)
        elif isinstance(v, list):
            for x in v:
                check(x)

    check(value)
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def request_digest(body: dict[str, Any]) -> str:
    """sha256(JCS(functional body without request_digest))."""
    return hashlib.sha256(canonical({k: v for k, v in body.items() if k != "request_digest"})).hexdigest()
