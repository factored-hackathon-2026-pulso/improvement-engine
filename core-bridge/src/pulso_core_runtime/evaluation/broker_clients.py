"""Real HTTP clients of the lab-broker for the arm runner (annex D.3 artifacts, plan D.4 bank, CAP-29 / A03).

* `BrokerArtifactPort` -> `GET /internal/v1/broker/artifacts/{id}` (`scope=artifact_read`).
* `BrokerSandboxClient` -> `/internal/v1/broker/sandbox/*` (`scope=sandbox`, purpose `sandbox_open|act|read|close`).

Every HTTP attempt mints a FRESH service JWT (`aud=lab-broker`, new `jti`) from the claims the caller supplies:
`mint(claims) -> JWS` adds `iss/aud/sub/iat/exp/jti`; the identity (`tenant_id`, `job_id`, `binding_ref`) comes from
the arm request, never from tool arguments. Failure semantics are conservative: before an effect is certain the
error is `SandboxUnavailable` (`failed_infra`); after a send whose answer may be lost it is `SandboxTimeout`
(`unknown`, resolved by readback)."""

from __future__ import annotations

import hashlib
import re
from collections.abc import Callable, Mapping
from typing import Any

import httpx
from agent_core.domain.json import canonical_bytes

from pulso_core_runtime.evaluation.sandbox_port import (
    ActionResult,
    SandboxSession,
    SandboxTimeout,
    SandboxUnavailable,
)

BROKER = "/internal/v1/broker"
Mint = Callable[[dict[str, Any]], str]
MAX_ARTIFACT_BYTES = 1 << 20
_HEX64 = re.compile(r"^[0-9a-f]{64}$")


class _Http:
    def __init__(self, base_url: str, mint: Mint, http: httpx.Client | None, timeout_s: float) -> None:
        self._http = http or httpx.Client(timeout=timeout_s)
        self._base, self._mint, self._timeout = base_url.rstrip("/"), mint, timeout_s

    def request(self, method: str, path: str, claims: dict[str, Any], *, json: Any = None,
                idempotency_key: str | None = None) -> httpx.Response:
        """One attempt, one token. Raises `httpx.TimeoutException` / `httpx.TransportError` untouched."""
        headers = {"Authorization": "Bearer " + self._mint(claims)}
        if claims.get("binding_ref"):
            headers["X-Pulso-Binding-Ref"] = str(claims["binding_ref"])
        if idempotency_key:
            headers["Idempotency-Key"] = idempotency_key
        return self._http.request(method, self._base + path, json=json, headers=headers, timeout=self._timeout)


class BrokerArtifactPort:
    """`ArtifactPort`: the sealed scenario manifest by reference. Fails closed with `LookupError` (the arm runner
    reports `failed_infra manifest_missing`) on any transport error, non-200, malformed envelope, digest mismatch,
    oversize or final-locked artifact, or a non-object JSON content."""

    def __init__(self, base_url: str, mint: Mint, *, http: httpx.Client | None = None,
                 timeout_s: float = 30.0) -> None:
        self._h = _Http(base_url, mint, http, timeout_s)

    def artifact_get(self, ref: str, tenant_id: str, binding_ref: str) -> dict[str, Any]:
        claims = {"scope": "artifact_read", "purpose": "artifact_read", "tenant_id": tenant_id,
                  "binding_ref": binding_ref}
        try:
            resp = self._h.request("GET", f"{BROKER}/artifacts/{ref}", claims)
        except httpx.HTTPError as exc:
            raise LookupError("artifact_unavailable") from exc
        if resp.status_code != 200:
            raise LookupError(f"artifact_http_{resp.status_code}")
        try:
            body = resp.json()
        except ValueError as exc:
            raise LookupError("artifact_malformed") from exc
        meta = body.get("artifact") if isinstance(body, dict) else None
        if not isinstance(meta, dict) or body.get("encoding") != "json" or not isinstance(body.get("content"), dict):
            raise LookupError("artifact_malformed")
        if meta.get("final_locked") or "final_locked" in str(meta.get("media_type", "")):
            raise LookupError("artifact_final_locked")  # oracle/gold never reaches the arm runner
        size = body.get("byte_length")
        if not isinstance(size, int) or size > MAX_ARTIFACT_BYTES:
            raise LookupError("artifact_too_large")
        declared = str(meta.get("digest", "")).removeprefix("sha256:")
        if not _HEX64.match(declared) or hashlib.sha256(canonical_bytes(body["content"])).hexdigest() != declared:
            raise LookupError("artifact_digest_mismatch")  # a manifest without a verifiable digest is not sealed
        content: dict[str, Any] = body["content"]
        return content


_SESSION_BODY = ("campaign_ref", "case_ref", "arm", "repetition")


class BrokerSandboxClient:
    """`EvaluationSandboxPort` over the Codex bank. `open(..., context)` carries the arm identity
    (`tenant_id`, `job_id`, `campaign_ref`, `case_ref`, `arm`, `repetition`); the session remembers it for the
    remaining calls. Without a tenant no token is minted and nothing is sent."""

    def __init__(self, base_url: str, mint: Mint, *, http: httpx.Client | None = None,
                 timeout_s: float = 30.0) -> None:
        self._h = _Http(base_url, mint, http, timeout_s)
        self._ctx: dict[str, dict[str, Any]] = {}

    # -- plumbing -------------------------------------------------------------------------------------
    def _claims(self, session_ref: str, purpose: str) -> dict[str, Any]:
        ctx = self._ctx.get(session_ref)
        if ctx is None:  # a session this client never opened has no identity to mint a token from
            raise SandboxUnavailable("sandbox_session_unknown")
        return {"scope": "sandbox", "purpose": purpose, "tenant_id": ctx["tenant_id"],
                "job_id": ctx.get("job_id"), "binding_ref": ctx["binding_ref"]}

    @staticmethod
    def _session(body: Any, fallback_ref: str | None = None) -> SandboxSession:
        try:
            return SandboxSession(str(body.get("session_ref", fallback_ref)), int(body["revision"]),
                                  str(body["initial_state_digest"]))
        except (AttributeError, KeyError, TypeError, ValueError) as exc:
            raise SandboxUnavailable("sandbox_malformed") from exc

    @staticmethod
    def _action(body: Any) -> ActionResult:
        try:
            return ActionResult(int(body["revision"]), str(body["effect_receipt"]), body["result"],
                                str(body["state_digest"]))
        except (AttributeError, KeyError, TypeError, ValueError) as exc:
            raise SandboxTimeout("sandbox_malformed_after_send") from exc  # an effect may have happened

    # -- port -------------------------------------------------------------------------------------------
    def open(self, binding_ref: str, seed_manifest_ref: str, context: Mapping[str, Any] | None = None) -> SandboxSession:
        ctx = dict(context or {})
        if not ctx.get("tenant_id"):
            raise SandboxUnavailable("sandbox_tenant_missing")
        body = {**{k: ctx.get(k) for k in _SESSION_BODY}, "seed_manifest_ref": seed_manifest_ref}
        claims = {"scope": "sandbox", "purpose": "sandbox_open", "tenant_id": ctx["tenant_id"],
                  "job_id": ctx.get("job_id"), "binding_ref": binding_ref}
        try:
            resp = self._h.request("POST", f"{BROKER}/sandbox/sessions", claims, json=body)
        except httpx.HTTPError as exc:  # a session that was created but not answered is garbage, not an effect
            raise SandboxUnavailable("sandbox_unavailable") from exc
        if resp.status_code != 200:
            raise SandboxUnavailable(f"sandbox_http_{resp.status_code}")
        try:
            session = self._session(resp.json())
        except ValueError as exc:
            raise SandboxUnavailable("sandbox_malformed") from exc
        self._ctx[session.session_ref] = {**ctx, "binding_ref": binding_ref}
        return session

    def reset(self, session: SandboxSession) -> SandboxSession:
        try:
            resp = self._h.request("POST", f"{BROKER}/sandbox/sessions/{session.session_ref}/reset",
                                   self._claims(session.session_ref, "sandbox_open"), json={})
        except httpx.HTTPError as exc:
            raise SandboxUnavailable("sandbox_unavailable") from exc
        if resp.status_code != 200:
            raise SandboxUnavailable(f"sandbox_http_{resp.status_code}")
        return self._session(resp.json(), session.session_ref)

    def act(self, session: SandboxSession, action_key: str, expected_revision: int,
            action: dict[str, Any]) -> ActionResult:
        try:
            resp = self._h.request(
                "POST", f"{BROKER}/sandbox/sessions/{session.session_ref}/actions",
                self._claims(session.session_ref, "sandbox_act"), idempotency_key=action_key,
                json={"action_key": action_key, "expected_revision": expected_revision, "action": action})
        except httpx.HTTPError as exc:  # sent, answer unknown: the engine verifies by readback
            raise SandboxTimeout("sandbox_timeout_after_send") from exc
        if resp.status_code >= 500:
            raise SandboxTimeout(f"sandbox_http_{resp.status_code}")
        if resp.status_code != 200:
            raise SandboxUnavailable(f"sandbox_http_{resp.status_code}")  # 4xx: refused, no effect
        try:
            return self._action(resp.json())
        except ValueError as exc:
            raise SandboxTimeout("sandbox_malformed_after_send") from exc

    def readback(self, session: SandboxSession, action_key: str) -> ActionResult | None:
        try:
            resp = self._h.request("GET", f"{BROKER}/sandbox/sessions/{session.session_ref}/actions/{action_key}",
                                   self._claims(session.session_ref, "sandbox_read"))
        except httpx.HTTPError as exc:
            raise SandboxTimeout("readback_unavailable") from exc
        if resp.status_code == 404:
            return None
        if resp.status_code != 200:
            raise SandboxTimeout(f"readback_http_{resp.status_code}")  # not proof of absence
        try:
            return self._action(resp.json())
        except ValueError as exc:
            raise SandboxTimeout("sandbox_malformed_after_send") from exc

    def close(self, session: SandboxSession, reason: str) -> str:
        try:
            resp = self._h.request("POST", f"{BROKER}/sandbox/sessions/{session.session_ref}/close",
                                   self._claims(session.session_ref, "sandbox_close"), json={"reason": reason})
        except httpx.HTTPError as exc:
            raise SandboxUnavailable("sandbox_unavailable") from exc
        if resp.status_code != 200:
            raise SandboxUnavailable(f"sandbox_http_{resp.status_code}")
        try:
            ref = resp.json().get("final_state_ref")
        except (ValueError, AttributeError) as exc:
            raise SandboxUnavailable("sandbox_malformed") from exc
        if not isinstance(ref, str) or not ref:
            raise SandboxUnavailable("sandbox_malformed")
        self._ctx.pop(session.session_ref, None)
        return ref
