"""Handlers for the L3a routes of the isolated `/internal/v1` app. Auth (audience, purpose, jti) is already
enforced by `build_internal_app`; handlers add the tenant checks. Wire with `make_handlers(...)` and pass the
result as `build_internal_app(..., handlers=...)`."""

from __future__ import annotations

import uuid
from collections.abc import Callable
from typing import Any

from fastapi import Request
from fastapi.responses import JSONResponse
from pydantic import BaseModel, ConfigDict

from pulso_core_runtime.credentials.issuer import CredentialIssuer
from pulso_core_runtime.internal.auth import Claims
from pulso_core_runtime.invoke.errors import BridgeError
from pulso_core_runtime.invoke.service import InvokeService, error_body

Handler = Callable[[Request, Claims], Any]


def _trace(request: Request) -> str:
    parts = request.headers.get("traceparent", "").split("-")
    return parts[1] if len(parts) == 4 and len(parts[1]) == 32 else uuid.uuid4().hex


def _respond(status: int, body: dict[str, Any], trace: str) -> JSONResponse:
    if "code" in body:
        body = {**body, "trace_id": trace}
    return JSONResponse(body, status_code=status)


class CredentialRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    tenant_id: str
    role: str
    purpose: str


def make_handlers(service: InvokeService, issuer: CredentialIssuer | None = None) -> dict[str, Handler]:
    async def invoke(request: Request, claims: Claims) -> JSONResponse:
        trace = _trace(request)
        try:
            raw = await request.json()
        except ValueError:
            return _respond(*_bad(trace), trace)
        if not isinstance(raw, dict):
            return _respond(*_bad(trace), trace)
        job = claims.raw.get("job_id")
        if not isinstance(job, str) or job != raw.get("job_id"):  # A03 (i): the signed job is the invoked job
            return _respond(403, error_body(BridgeError("pulso:auth_denied", 403, details={"reason": "job_mismatch"}),
                                            trace), trace)
        out = await service.invoke(claims.tenant_id, request.headers.get("idempotency-key"), raw)
        return _respond(out.status, out.body, trace)

    async def read(request: Request, claims: Claims) -> JSONResponse:
        trace = _trace(request)
        out = await service.read(claims.tenant_id, request.path_params["task_id"])
        return _respond(out.status, out.body, trace)

    async def issue(request: Request, claims: Claims) -> JSONResponse:
        trace = _trace(request)
        assert issuer is not None
        raw = await request.body()
        try:
            try:
                req = CredentialRequest.model_validate_json(raw)
            except ValueError:
                raise BridgeError("pulso:invalid_request", 422) from None
            result = issuer.issue(claims_tenant=claims.tenant_id, tenant_id=req.tenant_id, role=req.role,
                                  purpose=req.purpose)
        except BridgeError as exc:
            return _respond(exc.status, error_body(exc, trace), trace)
        return JSONResponse(result, headers={"Cache-Control": "no-store"})

    handlers: dict[str, Handler] = {"POST /core-tasks/invoke": invoke, "GET /core-tasks/{task_id}": read}
    if issuer is not None:
        handlers["POST /core-credentials/issue"] = issue
    return handlers


def _bad(trace: str) -> tuple[int, dict[str, Any]]:
    return 422, error_body(BridgeError("pulso:invalid_request", 422), trace)
