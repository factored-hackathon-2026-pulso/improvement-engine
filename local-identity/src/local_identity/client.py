"""Thin control-api -> human-issuer client (stand-in / Codex adapter reference).

Mints a fresh service JWT (new `jti`, TTL 60 s, `aud=human-issuer`, per-route `purpose`) for every HTTP attempt,
posts the DTO and returns typed results. Retrying keeps the business nonce only when the caller wants a replay
to be rejected; a new attempt needs a new nonce. Secrets (`assertion`, `authorization_jws`) are sensitive backend
values: never log, persist, or hand them to a browser."""

from __future__ import annotations

import hmac
import json
import uuid
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import httpx
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from local_identity import CALLER_AUDIENCE, CALLER_ISSUER, COMMAND_PURPOSE, SESSION_PURPOSE
from local_identity.keys import b64url_decode, jws_compact

SESSION_PATH = "/internal/v1/human/session-assertions/issue"
COMMAND_PATH = "/internal/v1/human/command-authorizations/issue"
TOKEN_TTL_S = 60


class IssuerError(Exception):
    """Issuer refusal or outage. `code` is the D.1 envelope code (e.g. `pulso:role_not_allowed`)."""

    def __init__(
        self,
        code: str,
        status: int,
        *,
        retryable: bool = False,
        trace_id: str = "",
        details: dict[str, Any] | None = None,
    ) -> None:
        super().__init__(f"{code} ({status})")
        self.code, self.status, self.retryable, self.trace_id = code, status, retryable, trace_id
        self.details = details or {}


@dataclass(frozen=True)
class ServiceSigner:
    """The control-api side service-JWT key (`issuer` -> `audience`)."""

    kid: str
    private_key: Ed25519PrivateKey = field(repr=False)
    issuer: str = CALLER_ISSUER
    audience: str = CALLER_AUDIENCE

    @classmethod
    def from_file(cls, path: Path, **kw: str) -> ServiceSigner:
        data = json.loads(Path(path).read_bytes())
        return cls(data["kid"], Ed25519PrivateKey.from_private_bytes(b64url_decode(data["key"])), **kw)

    @classmethod
    def generate(cls, kid: str, **kw: str) -> ServiceSigner:
        return cls(kid, Ed25519PrivateKey.generate(), **kw)

    def sign_claims(self, claims: dict[str, Any]) -> str:
        return jws_compact(
            self.private_key, kid=self.kid, typ="JWT", payload=json.dumps(claims, separators=(",", ":")).encode()
        )


@dataclass(frozen=True)
class SessionAssertion:
    assertion: str = field(repr=False)
    kid: str
    exp: int


@dataclass(frozen=True)
class CommandAuthorization:
    authorization_jws: str = field(repr=False)
    kid: str
    exp: int
    metadata: dict[str, Any] = field(default_factory=dict)


class LocalIdentityClient:
    def __init__(
        self,
        base_url: str,
        signer: ServiceSigner,
        *,
        tenant_id: str | None = None,
        subject: str = "control-api-worker",
        http_client: httpx.Client | None = None,
        now: Callable[[], datetime] = lambda: datetime.now(UTC),
        tenant_override: str | None = None,
        timeout_s: float = 5.0,
    ) -> None:
        self.signer, self._base, self._subject, self._now = signer, base_url.rstrip("/"), subject, now
        self._tenant, self._override = tenant_id, tenant_override
        self._http = http_client or httpx.Client(timeout=timeout_s)

    def mint(self, purpose: str, tenant_id: str | None = None) -> str:
        """A fresh service token (new `jti`) for one HTTP attempt."""
        tenant = self._override or tenant_id or self._tenant
        if not tenant:
            raise ValueError("tenant_id required to mint a service token")
        iat = int(self._now().timestamp())
        return self.signer.sign_claims(
            {
                "iss": self.signer.issuer,
                "aud": self.signer.audience,
                "sub": self._subject,
                "tenant_id": tenant,
                "purpose": purpose,
                "iat": iat,
                "exp": iat + TOKEN_TTL_S,
                "jti": uuid.uuid4().hex,
            }
        )

    def _post(self, path: str, purpose: str, tenant_id: str, body: dict[str, Any]) -> dict[str, Any]:
        headers = {"Authorization": f"Bearer {self.mint(purpose, tenant_id)}"}
        try:
            response = self._http.post(f"{self._base}{path}", headers=headers, json=body)
        except httpx.HTTPError:
            raise IssuerError("pulso:issuer_unavailable", 503, retryable=True) from None
        data = response.json() if response.content else {}
        if response.status_code != 200:
            raise IssuerError(
                str(data.get("code", "pulso:http_error")),
                response.status_code,
                retryable=bool(data.get("retryable", False)),
                trace_id=str(data.get("trace_id", "")),
                details=data.get("details") or {},
            )
        return data

    def session_assertion(
        self, *, tenant_id: str, actor_ref: str, session_intent_ref: str, nonce: str
    ) -> SessionAssertion:
        data = self._post(
            SESSION_PATH,
            SESSION_PURPOSE,
            tenant_id,
            {"tenant_id": tenant_id, "actor_ref": actor_ref, "session_intent_ref": session_intent_ref, "nonce": nonce},
        )
        return SessionAssertion(data["assertion"], data["kid"], data["exp"])

    def command_authorization(
        self,
        *,
        tenant_id: str,
        actor_ref: str,
        command_ref: str,
        operation: str,
        target: dict[str, Any],
        challenge_ref: str,
        nonce: str,
    ) -> CommandAuthorization:
        data = self._post(
            COMMAND_PATH,
            COMMAND_PURPOSE,
            tenant_id,
            {
                "tenant_id": tenant_id,
                "actor_ref": actor_ref,
                "command_ref": command_ref,
                "operation": operation,
                "target": target,
                "challenge_ref": challenge_ref,
                "nonce": nonce,
            },
        )
        return CommandAuthorization(data["authorization_jws"], data["kid"], data["exp"], data.get("metadata", {}))


__all__ = ["CommandAuthorization", "IssuerError", "LocalIdentityClient", "ServiceSigner", "SessionAssertion"]


class BindingMismatch(Exception):
    """The authorization JWS is not bound to the operation/target the caller is about to execute."""


def assert_bound(
    authorization_jws: str,
    *,
    tenant_id: str,
    actor_ref: str,
    command_ref: str,
    operation: str,
    target: dict[str, Any],
    challenge_ref: str,
    now: datetime | None = None,
) -> None:
    """Port-side binding check (Codex `HumanAuthorizationPort` / stand-in).

    WHERE ENFORCEMENT MUST LIVE: Core verifies the signature, `auth.level`, role and `exp` of the Principal and
    IGNORES every other attr, so Core alone cannot tell "approve prop-1@3" from "publish prop-2@9". The port MUST,
    in this order and on the exact bytes it then sends to Core: (1) load its durable intention and atomically
    transition it to consumed (single use; the Principal has no jti and is replayable for its 60 s lifetime),
    (2) call this function with values taken from that intention, never from the request, (3) pass `now` so a
    stale credential is refused locally, (4) dispatch to Core with this same JWS. Skipping (2) lets an approval
    for one operation/hash/revision be replayed against another.

    Recomputes the binding digest from the intention and requires every signed attr, the actor, the step-up level,
    the simulated marker, the role required by the operation and (when `now` is given) `exp` to agree. The
    signature is verified by Core's verifier, not here: this function trusts nothing it cannot recompute."""
    from pydantic import ValidationError

    from local_identity.principal import POLICY, CommandRequest, binding_digest, binding_of

    try:
        payload = json.loads(b64url_decode(authorization_jws.split(".")[1]))
        req = CommandRequest.model_validate(
            {
                "tenant_id": tenant_id,
                "actor_ref": actor_ref,
                "command_ref": command_ref,
                "operation": operation,
                "target": target,
                "challenge_ref": challenge_ref,
                "nonce": "x" * 16,
            }
        )
        attrs, auth, roles = payload["attrs"], payload["auth"], payload["roles"]
        policy = POLICY[operation]
        if policy[0] != req.target.kind or not (
            isinstance(attrs, dict) and isinstance(auth, dict) and isinstance(roles, list)
        ):
            raise ValueError
        expected = binding_digest(binding_of(req))
        expected_attrs = {
            "actor": "human",
            "tenant": tenant_id,
            "command_ref": command_ref,
            "operation": operation,
            "challenge_ref": challenge_ref,
            "target_kind": req.target.kind,
            **{k: str(v) for k, v in req.target.model_dump(mode="json", exclude={"kind"}).items()},
        }
        exp = datetime.fromisoformat(payload["exp"]) if now is not None else None
    except (IndexError, KeyError, TypeError, ValueError, ValidationError, AttributeError):
        raise BindingMismatch("malformed") from None
    if (
        not isinstance(attrs.get("binding_digest"), str)
        or not hmac.compare_digest(attrs["binding_digest"].encode(), expected.encode())
        or payload.get("id") != actor_ref
        or any(attrs.get(k) != v for k, v in expected_attrs.items())
        or auth.get("level") != "step_up"
        or auth.get("simulated") is not True
        or policy[1] not in roles
        or (operation == "revoke") != ("admin" in roles)
    ):
        raise BindingMismatch("binding_digest")
    if now is not None and exp is not None and now >= exp:
        raise BindingMismatch("expired")
