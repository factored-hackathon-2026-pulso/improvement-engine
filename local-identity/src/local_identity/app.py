"""The two private issuer routes. D.1 error envelope `{schema_version, code, retryable, trace_id, details}`.

Trust model (plan P1/P4): the issuer has no Pulso state. It checks the authenticated caller (service JWT bound to
`aud=human-issuer`), the actor allowlist (server configuration, never the request), the local/simulated profile
and digest formats. Codex alone verifies the browser session, the durable intention, challenge, freshness and
hash/revision before dispatch."""

from __future__ import annotations

import json
import re
import uuid
from collections.abc import Callable
from datetime import UTC, datetime, timedelta
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from pydantic import ValidationError
from starlette.exceptions import HTTPException as StarletteHTTPException

from local_identity import COMMAND_PURPOSE, ISSUER_NAME, SESSION_AUDIENCE, SESSION_PURPOSE
from local_identity.auth import AuthError, Claims, ServiceJwtVerifier
from local_identity.config import Config, Identity
from local_identity.principal import (
    BASE_ROLES,
    POLICY,
    CommandRequest,
    SessionRequest,
    binding_digest,
    binding_of,
    human_principal,
)
from local_identity.replay import ReplayStore

SESSION_PATH = "/internal/v1/human/session-assertions/issue"
COMMAND_PATH = "/internal/v1/human/command-authorizations/issue"
MAX_BODY_BYTES = 64 * 1024


def envelope(
    code: str, *, status: int, retryable: bool = False, trace_id: str = "", details: dict[str, Any] | None = None
) -> JSONResponse:
    return JSONResponse(
        {"schema_version": "1", "code": code, "retryable": retryable, "trace_id": trace_id, "details": details or {}},
        status_code=status,
    )


class Denied(Exception):
    def __init__(self, code: str, status: int, details: dict[str, Any] | None = None) -> None:
        super().__init__(code)
        self.code, self.status, self.details = code, status, details or {}


def _trace_id(request: Request) -> str:
    parts = request.headers.get("traceparent", "").split("-")
    if len(parts) == 4 and re.fullmatch(r"[0-9a-f]{32}", parts[1]):
        return parts[1]
    return uuid.uuid4().hex


def build_app(
    config: Config, *, now: Callable[[], datetime] = lambda: datetime.now(UTC), replay: ReplayStore | None = None
) -> FastAPI:
    replay = replay or ReplayStore(config.replay_db)
    verifier = ServiceJwtVerifier(config.service_keys, replay, now, config.clock_skew_s)
    app = FastAPI(title="local-identity", docs_url=None, redoc_url=None, openapi_url=None)

    @app.exception_handler(StarletteHTTPException)
    async def _http(request: Request, exc: StarletteHTTPException) -> JSONResponse:
        return envelope(
            "pulso:not_found" if exc.status_code == 404 else "pulso:http_error",
            status=exc.status_code,
            trace_id=_trace_id(request),
        )

    @app.exception_handler(Exception)
    async def _boom(request: Request, exc: Exception) -> JSONResponse:
        return envelope("pulso:internal_error", status=500, retryable=True, trace_id=_trace_id(request))

    @app.get("/healthz")
    def healthz() -> dict[str, str]:
        return {"status": "ok"}

    @app.get("/readyz")
    def readyz() -> dict[str, str]:
        return {"status": "ready"}

    def identity_for(claims: Claims, tenant_id: str, actor_ref: str) -> Identity:
        if claims.tenant_id != tenant_id:
            raise Denied("pulso:tenant_mismatch", 403)
        identity = config.identities.get(actor_ref)
        if identity is None:
            raise Denied("pulso:actor_not_allowed", 403)
        if identity.tenant_id != tenant_id:
            raise Denied("pulso:tenant_mismatch", 403)
        return identity

    def consume_nonce(purpose: str, tenant_id: str, nonce: str, moment: datetime) -> None:
        retain = int(moment.timestamp()) + config.nonce_retention_s
        if not replay.consume_nonce(purpose, tenant_id, nonce, retain_until=retain, now=int(moment.timestamp())):
            raise Denied("pulso:nonce_replayed", 409)

    def issue_session(claims: Claims, req: SessionRequest) -> dict[str, Any]:
        identity_for(claims, req.tenant_id, req.actor_ref)
        moment = now()
        consume_nonce(SESSION_PURPOSE, req.tenant_id, req.nonce, moment)
        iat = int(moment.timestamp())
        payload = {
            "iss": ISSUER_NAME,
            "aud": SESSION_AUDIENCE,
            "sub": req.actor_ref,
            "tenant_id": req.tenant_id,
            "nonce": req.nonce,
            "session_intent_ref": req.session_intent_ref,
            "iat": iat,
            "exp": iat + config.session_ttl_s,
            "jti": uuid.uuid4().hex,
            "auth_level": "session",
            "auth_at": iat,
        }
        token = config.session_signer.sign(typ="JWT", payload=json.dumps(payload, separators=(",", ":")).encode())
        return {"assertion": token, "kid": config.session_signer.kid, "exp": payload["exp"]}

    def issue_command(claims: Claims, req: CommandRequest) -> dict[str, Any]:
        policy = POLICY.get(req.operation)
        if policy is None or policy[0] != req.target.kind:
            raise Denied("pulso:invalid_request", 422, {"fields": ["operation", "target"]})
        identity = identity_for(claims, req.tenant_id, req.actor_ref)
        required = policy[1]
        if required not in identity.roles:
            raise Denied("pulso:role_not_allowed", 403)
        moment = now()
        consume_nonce(COMMAND_PURPOSE, req.tenant_id, req.nonce, moment)
        roles = [r for r in BASE_ROLES if r in identity.roles] + (["admin"] if req.operation == "revoke" else [])
        exp = moment + timedelta(seconds=config.step_up_ttl_s)
        principal = human_principal(req, roles=roles, now=moment, exp=exp)
        jws = config.human_signer.sign(
            typ="principal+jws", payload=json.dumps(principal, separators=(",", ":")).encode()
        )
        return {
            "authorization_jws": jws,
            "kid": config.human_signer.kid,
            "exp": int(exp.timestamp()),
            "metadata": {
                "auth_simulated": True,
                "operation": req.operation,
                "binding_digest": binding_digest(binding_of(req)),
            },
        }

    async def handle(
        request: Request,
        purpose: str,
        model: type[Any],
        issue: Callable[[Claims, Any], dict[str, Any]],
    ) -> JSONResponse:
        trace = _trace_id(request)
        scheme, _, token = request.headers.get("authorization", "").partition(" ")
        try:
            if scheme.lower() != "bearer" or not token:
                raise AuthError("missing_token")
            claims = verifier.verify(token, purpose=purpose)
        except AuthError as exc:
            if exc.reason == "jti_replayed":
                return envelope("pulso:service_token_replayed", status=401, trace_id=trace)
            return envelope(
                "pulso:auth_denied" if exc.status == 403 else "pulso:auth_invalid",
                status=exc.status,
                trace_id=trace,
                details={"reason": exc.reason},
            )
        declared = request.headers.get("content-length", "")
        if declared.isdigit() and int(declared) > MAX_BODY_BYTES:
            return envelope("pulso:payload_too_large", status=413, trace_id=trace)
        raw = b""
        async for chunk in request.stream():  # capped read: never buffer an unbounded body
            raw += chunk
            if len(raw) > MAX_BODY_BYTES:
                return envelope("pulso:payload_too_large", status=413, trace_id=trace)
        try:
            req = model.model_validate_json(raw)
        except ValidationError as exc:
            fields = sorted({".".join(str(p) for p in e["loc"]) for e in exc.errors()})
            return envelope("pulso:invalid_request", status=422, trace_id=trace, details={"fields": fields})
        try:
            return JSONResponse(issue(claims, req))
        except Denied as exc:
            return envelope(exc.code, status=exc.status, trace_id=trace, details=exc.details)

    @app.post(SESSION_PATH, include_in_schema=False)
    async def session_route(request: Request) -> JSONResponse:
        return await handle(request, SESSION_PURPOSE, SessionRequest, issue_session)

    @app.post(COMMAND_PATH, include_in_schema=False)
    async def command_route(request: Request) -> JSONResponse:
        return await handle(request, COMMAND_PURPOSE, CommandRequest, issue_command)

    return app
