"""`register()` module for `GET /core-state/aliases/{agent_id}/{alias}` and `POST /core-authoring/dry-run`.

Wiring: `register(handlers, AuthoringDeps(authoring_service))` (done in `main._compose`).
Auth (service JWT, audience, purpose `alias_read` / `authoring_dry_run`, jti) is enforced by `build_internal_app`
before a handler runs. Errors use the D.1 envelope."""

from __future__ import annotations

import json
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from fastapi import Request
from fastapi.responses import JSONResponse
from starlette.concurrency import run_in_threadpool

from pulso_core_runtime.authoring.service import AuthoringService
from pulso_core_runtime.internal.auth import Claims
from pulso_core_runtime.invoke.errors import BridgeError


@dataclass
class AuthoringDeps:
    service: AuthoringService


def _reject_constant(name: str) -> Any:
    raise ValueError(name)


def _fail(exc: BridgeError, request: Request) -> JSONResponse:
    from pulso_core_runtime.internal.app import _trace_id, envelope

    return envelope(exc.code, retryable=exc.retryable, details=exc.details, trace_id=_trace_id(request),
                    status=exc.status)


def build_handlers(deps: AuthoringDeps) -> dict[str, Callable[[Request, Claims], Any]]:
    async def alias(request: Request, claims: Claims) -> JSONResponse:
        try:
            body = await run_in_threadpool(deps.service.alias_state, request.path_params["agent_id"],
                                           request.path_params["alias"])
        except BridgeError as exc:
            return _fail(exc, request)
        return JSONResponse(body, headers={"Cache-Control": "no-store"})

    async def dry_run(request: Request, claims: Claims) -> JSONResponse:
        try:
            raw = json.loads(await request.body(), parse_constant=_reject_constant)
        except (ValueError, RecursionError):  # bad UTF-8, bad JSON, NaN/Infinity, absurd nesting
            return _fail(BridgeError("pulso:invalid_request", 422), request)
        if not isinstance(raw, dict):
            return _fail(BridgeError("pulso:invalid_request", 422), request)
        try:
            body = await run_in_threadpool(deps.service.dry_run, claims.tenant_id or "", raw)
        except BridgeError as exc:
            return _fail(exc, request)
        return JSONResponse(body, headers={"Cache-Control": "no-store"})

    return {"GET /core-state/aliases/{agent_id}/{alias}": alias, "POST /core-authoring/dry-run": dry_run}


def register(handlers: dict[str, Callable[[Request, Claims], Any]], deps: AuthoringDeps) -> None:
    handlers.update(build_handlers(deps))
