"""Sim-only EdDSA JWS minting/verification (header exactly {alg,kid,typ}, typ=principal+jws).

The key is derived from a fixed public label (kid starts with `sim-`): it only exists for simulators and tests,
never for staging or production (CAP-63). Independent of agent_core and of any Pulso adapter code."""

from __future__ import annotations

import base64
import hashlib
import json
from datetime import datetime, timedelta, timezone
from typing import Any

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey

ALG = "EdDSA"
PRINCIPAL_TYP = "principal+jws"
SIM_KID = "sim-staff-1"
MAX_TOKEN_CHARS = 8192
SIM_EPOCH = datetime(2026, 1, 1, tzinfo=timezone.utc)


def b64url_encode(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def b64url_decode(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def sim_private_key() -> Ed25519PrivateKey:
    return Ed25519PrivateKey.from_private_bytes(hashlib.sha256(b"pulso registry-sim TEST ONLY key: staff").digest())


def sim_public_key() -> Ed25519PublicKey:
    return sim_private_key().public_key()


def _dumps(value: Any) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def iso(value: datetime) -> str:
    return value.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%fZ")


def sign(header: dict[str, Any], payload: dict[str, Any], key: Ed25519PrivateKey | None = None) -> str:
    key = key or sim_private_key()
    head, body = b64url_encode(_dumps(header)), b64url_encode(_dumps(payload))
    return f"{head}.{body}.{b64url_encode(key.sign(f'{head}.{body}'.encode('ascii')))}"


def principal_payload(pid: str | None, ptype: str, roles: list[str], *, human: bool = False, step_up: bool = False,
                      exp: datetime | None = None, now: datetime = SIM_EPOCH) -> dict[str, Any]:
    auth: dict[str, Any] = {"level": "step_up" if step_up else "session", "at": iso(now)}
    if step_up:
        auth["simulated"] = True
    return {"type": ptype, "id": pid, "roles": roles, "scopes": [], "attrs": {"actor": "human"} if human else {},
            "auth": auth, "exp": iso(exp or now + timedelta(days=3650))}


def issue(actor: str, *, exp: datetime | None = None, kid: str = SIM_KID, typ: str = PRINCIPAL_TYP,
          key: Ed25519PrivateKey | None = None) -> str:
    """actor: bot | human | admin | customer | advisor | human_session (supervisor without step_up)."""
    table = {
        "bot": ("constructor-bot", "builder", ["constructor"], False, False),
        "human": ("ana", "builder", ["constructor", "aprobador"], True, True),
        "human_session": ("ana", "builder", ["constructor", "aprobador"], True, False),
        "admin": ("root", "builder", ["constructor", "aprobador", "admin"], True, True),
        "customer": ("cust-001", "customer", [], False, False),
        "advisor": ("adv-7", "advisor", [], False, False),
        "bot_aprobador": ("constructor-bot", "builder", ["constructor", "aprobador"], False, False),
        "approver_only": ("ana", "builder", ["aprobador"], True, True),
    }
    pid, ptype, roles, human, step_up = table[actor]
    payload = principal_payload(pid, ptype, roles, human=human, step_up=step_up, exp=exp)
    return sign({"alg": ALG, "kid": kid, "typ": typ}, payload, key)


def verify(token: str) -> dict[str, Any]:
    """Raises ValueError on any doubt (fail closed). Returns the principal payload."""
    if not isinstance(token, str) or not token or len(token) > MAX_TOKEN_CHARS:
        raise ValueError("credential")
    head, body, sig = token.split(".")
    header = json.loads(b64url_decode(head))
    if not isinstance(header, dict) or set(header) != {"alg", "kid", "typ"}:
        raise ValueError("header")
    if header["alg"] != ALG or header["typ"] != PRINCIPAL_TYP or header["kid"] != SIM_KID:
        raise ValueError("header")
    sim_public_key().verify(b64url_decode(sig), f"{head}.{body}".encode("ascii"))
    payload = json.loads(b64url_decode(body))
    if not isinstance(payload, dict):
        raise ValueError("payload")
    return payload
