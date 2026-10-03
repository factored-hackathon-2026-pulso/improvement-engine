"""Isolated `/internal/v1` FastAPI sub-application (DR-34): own handlers and the D.1 error envelope
`{schema_version, code, retryable, trace_id, details}`; Core's global `problem+json` handlers never apply."""

from __future__ import annotations

import uuid
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from starlette.exceptions import HTTPException as StarletteHTTPException

from pulso_core_runtime.internal.auth import AuthError, Claims, ServiceJwtVerifier

CORE_BRIDGE = "core-bridge"
TENANT_EXEMPT = frozenset({"version_probe"})  # the only route with no tenant data
MAX_BODY_BYTES = 1024 * 1024  # invoke inputs are capped at 256 KiB; nothing legitimate is larger


@dataclass(frozen=True)
class Route:
    method: str
    path: str
    audience: str
    purposes: frozenset[str]
    handler: Callable[[Request, Claims], Any] | None = None  # None -> 501 until the owning package lands


# Route map (A03 class i). L3/L5 plug handlers in with `replace_handler`; auth is already enforced.
ROUTES: tuple[Route, ...] = (
    Route("POST", "/core-tasks/invoke", CORE_BRIDGE, frozenset({"core_task_invoke"})),
    Route("GET", "/core-tasks/{task_id}", CORE_BRIDGE, frozenset({"core_task_read"})),
    Route("GET", "/core-state/aliases", CORE_BRIDGE, frozenset({"alias_read"})),
    Route("POST", "/core-authoring/dry-run", CORE_BRIDGE, frozenset({"authoring_dry_run"})),
    Route("GET", "/version", CORE_BRIDGE, frozenset({"version_probe"})),
    Route("POST", "/core-credentials/issue", CORE_BRIDGE, frozenset({"credential_issue"})),
    Route("POST", "/evaluation/admissions", CORE_BRIDGE, frozenset({"evaluation_admit"})),
    # Static segments first: Starlette matches in registration order, so `by-key` is never read as an `{arm_id}`.
    Route("POST", "/evaluation/arms/run", CORE_BRIDGE, frozenset({"evaluation_arm_run"})),
    Route("GET", "/evaluation/arms/by-key/{key}", CORE_BRIDGE, frozenset({"evaluation_arm_read"})),
    Route("POST", "/evaluation/arms/{arm_id}/run", CORE_BRIDGE, frozenset({"evaluation_arm_run"})),
    Route("GET", "/evaluation/arms/{arm_id}", CORE_BRIDGE, frozenset({"evaluation_arm_read"})),
)


def envelope(code: str, *, retryable: bool = False, trace_id: str = "", details: dict[str, Any] | None = None,
             status: int = 400) -> JSONResponse:
    return JSONResponse({"schema_version": "1", "code": code, "retryable": retryable,
                         "trace_id": trace_id, "details": details or {}}, status_code=status)


def _trace_id(request: Request) -> str:
    parent = request.headers.get("traceparent", "")
    parts = parent.split("-")
    return parts[1] if len(parts) == 4 and len(parts[1]) == 32 else uuid.uuid4().hex


async def _read_capped(request: Request) -> bool:
    """Buffers the body (handlers then read the cached copy) refusing anything above `MAX_BODY_BYTES`, by
    declared length and by actual bytes (chunked bodies have no length)."""
    declared = request.headers.get("content-length")
    if declared is not None and (not declared.isdigit() or int(declared) > MAX_BODY_BYTES):
        return False
    chunks: list[bytes] = []
    size = 0
    async for chunk in request.stream():
        size += len(chunk)
        if size > MAX_BODY_BYTES:
            return False
        chunks.append(chunk)
    request._body = b"".join(chunks)
    return True


def build_internal_app(verifier: ServiceJwtVerifier, *, version_info: Callable[[], dict[str, Any]],
                       handlers: dict[str, Callable[[Request, Claims], Any]] | None = None,
                       l3: Any | None = None) -> FastAPI:
    app = FastAPI(title="pulso-internal", docs_url=None, redoc_url=None, openapi_url=None)
    handlers = dict(handlers or {})
    if l3 is not None:  # L3a/L3b composition (`invoke.wiring.build_l3`): invoke, read, credentials
        handlers.update(l3.handlers)
    handlers.setdefault("GET /version", lambda request, claims: version_info())

    @app.exception_handler(StarletteHTTPException)
    async def _http(request: Request, exc: StarletteHTTPException) -> JSONResponse:
        code = "pulso:not_found" if exc.status_code == 404 else "pulso:http_error"
        return envelope(code, trace_id=_trace_id(request), status=exc.status_code)

    @app.exception_handler(RequestValidationError)
    async def _invalid(request: Request, exc: RequestValidationError) -> JSONResponse:
        fields = sorted({".".join(str(p) for p in e["loc"][1:]) for e in exc.errors()})
        return envelope("pulso:invalid_request", trace_id=_trace_id(request), details={"fields": fields}, status=422)

    @app.exception_handler(Exception)
    async def _boom(request: Request, exc: Exception) -> JSONResponse:
        return envelope("pulso:internal_error", retryable=True, trace_id=_trace_id(request), status=500)

    def make(route: Route) -> Callable[..., Any]:
        async def endpoint(request: Request) -> Any:
            trace = _trace_id(request)
            header = request.headers.get("authorization", "")
            scheme, _, token = header.partition(" ")
            try:
                if scheme.lower() != "bearer" or not token:
                    raise AuthError("missing_token")
                claims = verifier.verify(token, audience=route.audience, purposes=route.purposes,
                                         require_tenant=not route.purposes <= TENANT_EXEMPT)
            except AuthError as exc:
                return envelope("pulso:auth_denied" if exc.status == 403 else "pulso:auth_invalid",
                                trace_id=trace, details={"reason": exc.reason}, status=exc.status)
            handler = route.handler or handlers.get(f"{route.method} {route.path}")
            if handler is None:
                return envelope("pulso:not_implemented", trace_id=trace, status=501)
            if not await _read_capped(request):
                return envelope("pulso:payload_too_large", trace_id=trace, status=413)
            result = handler(request, claims)
            if hasattr(result, "__await__"):
                result = await result
            return result if isinstance(result, JSONResponse) else JSONResponse(result)
        return endpoint

    for route in ROUTES:
        app.add_api_route(route.path, make(route), methods=[route.method], include_in_schema=False)
    return app
