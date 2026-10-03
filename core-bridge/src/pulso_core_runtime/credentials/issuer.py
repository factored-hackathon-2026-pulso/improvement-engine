"""Bot principal signing and `POST /internal/v1/core-credentials/issue` (CAP-33/CAP-04).

Two independent signers (staff for `/v1/registry`, identity for `/v1/runs`): one credential is never valid in
both verifiers. The (purpose, role) policy is versioned here, never chosen from model arguments. The JWS is
returned once and never persisted or logged."""

from __future__ import annotations

import json
from collections.abc import Callable
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from pulso_core_runtime.internal.auth import b64url_decode, b64url_encode
from pulso_core_runtime.invoke.errors import BridgeError

POLICY_VERSION = "1"
MAX_TTL = timedelta(minutes=15)
# (purpose, role) -> which signer. Humans and approve/publish/promote roles are not issuable here.
POLICY: dict[tuple[str, str], str] = {
    ("registry_write", "constructor"): "staff",
    ("core_task", "constructor"): "identity",
}


class PrincipalSigner:
    """Ed25519 signer of `principal+jws` credentials for one `kid`."""

    def __init__(self, kid: str, key: Ed25519PrivateKey) -> None:
        self.kid, self._key = kid, key

    def sign(self, principal: dict[str, Any]) -> str:
        from agent_core.domain import Principal, dumps

        canonical = dumps(Principal.model_validate(principal))  # validates shape, never signs garbage
        head = b64url_encode(json.dumps({"alg": "EdDSA", "kid": self.kid, "typ": "principal+jws"},
                                        separators=(",", ":")).encode())
        body = b64url_encode(canonical.encode())
        return f"{head}.{body}.{b64url_encode(self._key.sign(f'{head}.{body}'.encode('ascii')))}"

    def __repr__(self) -> str:
        return f"PrincipalSigner(kid={self.kid!r})"


def load_signer(path: Path) -> PrincipalSigner:
    """File `{"kid": str, "key": b64url(32-byte ed25519 seed)}`. Errors name the file, never the value."""
    try:
        data = json.loads(path.read_bytes())
        seed = b64url_decode(data["key"])
        if len(seed) != 32:
            raise ValueError
        return PrincipalSigner(str(data["kid"]), Ed25519PrivateKey.from_private_bytes(seed))
    except (OSError, KeyError, TypeError, ValueError):
        raise ValueError(f"pulso:credential_signing_unavailable: signer file unreadable ({path.name})") from None


def iso_z(moment: datetime) -> str:
    return moment.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"


def bot_principal(*, principal_id: str, roles: list[str], attrs: dict[str, str], now: datetime,
                  exp: datetime) -> dict[str, Any]:
    return {"type": "builder", "id": principal_id, "roles": roles, "scopes": [], "attrs": attrs,
            "auth": {"level": "session", "at": iso_z(now)}, "exp": iso_z(exp)}


class CredentialIssuer:
    def __init__(self, signers: dict[str, PrincipalSigner],
                 now: Callable[[], datetime] = lambda: datetime.now(UTC), ttl: timedelta = MAX_TTL) -> None:
        self._signers, self._now = signers, now
        self._ttl = min(ttl, MAX_TTL)

    def issue(self, *, claims_tenant: str | None, tenant_id: str, role: str, purpose: str) -> dict[str, Any]:
        if claims_tenant is not None and claims_tenant != tenant_id:
            raise BridgeError("pulso:tenant_mismatch", 403)
        which = POLICY.get((purpose, role))
        if which is None:
            raise BridgeError("pulso:credential_not_issuable", 403)
        signer = self._signers.get(which)
        if signer is None:
            raise BridgeError("pulso:credential_signing_unavailable", 503, retryable=True)
        now = self._now()
        exp = now + self._ttl
        principal = bot_principal(principal_id=f"pulso-constructor:{tenant_id}", roles=[role],
                                  attrs={"tenant": tenant_id}, now=now, exp=exp)
        try:
            jws = signer.sign(principal)
        except Exception:
            raise BridgeError("pulso:credential_signing_unavailable", 503, retryable=True) from None
        return {"jws": jws, "kid": signer.kid, "exp": int(exp.timestamp())}
