"""`pulso/bind_context` backend: resolves the `InvocationContext` and calls the control-api binding callback
(CAP-27). The ToolDef handler itself lives in `tools/bind.py` (L3b) and calls `BindingService.bind`.

Callback: `POST {PULSO_CONTROL_API_URL}/internal/v1/core-task-bindings`, JWT `iss=core-bridge, aud=control-api,
scope=binding`, `Idempotency-Key=command_key`, 5 s timeout, one retry on network error."""

from __future__ import annotations

import time
import uuid
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

import httpx

from pulso_core_runtime.internal.auth import sign_service_jwt
from pulso_core_runtime.invoke.context import ContextError, InvocationRegistry
from pulso_core_runtime.store.receipts import ReceiptStore

BINDING_PATH = "/internal/v1/core-task-bindings"
STAGE_FACTS = {"scout": "is_scout", "verifier": "is_verifier", "builder_design": "is_builder", "writer": "is_writer"}


@dataclass(frozen=True)
class BindResult:
    ok: bool
    reason: str | None = None
    facts: dict[str, bool] | None = None
    effect_unproven: bool = False  # True when absence of effect cannot be proven (timeout / 5xx)


class BindingService:
    def __init__(self, *, store: ReceiptStore, registry: InvocationRegistry, control_api_url: str, signing_key: Any,
                 kid: str, bridge_instance_id: str, transport: httpx.BaseTransport | None = None,
                 timeout_s: float = 5.0, clock: Callable[[], float] = time.time) -> None:
        self._store, self._registry = store, registry
        self._url, self._key, self._kid = control_api_url.rstrip("/") + BINDING_PATH, signing_key, kid
        self._instance, self._transport, self._timeout, self._clock = bridge_instance_id, transport, timeout_s, clock

    def _token(self, tenant_id: str) -> str:
        now = int(self._clock())
        return sign_service_jwt(self._key, kid=self._kid, claims={
            "iss": "core-bridge", "aud": "control-api", "sub": f"bridge:{self._instance}", "scope": "binding",
            "purpose": "core_task_binding", "tenant_id": tenant_id, "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex})

    def bind(self, *, run_id: str, principal_attrs: dict[str, str]) -> BindResult:
        try:
            ref = principal_attrs.get("task_binding_ref")
            if not ref:
                raise ContextError("pulso:context_missing")
            ctx = self._registry.lookup(ref)
            if (principal_attrs.get("tenant"), principal_attrs.get("job")) != (ctx.tenant_id, ctx.job_id):
                raise ContextError("pulso:context_mismatch")
        except ContextError as exc:
            return BindResult(False, exc.code)
        body = {"schema_version": "1", "tenant_id": ctx.tenant_id, "job_id": ctx.job_id,
                "command_key": ctx.command_key, "request_digest": ctx.request_digest, "attempt": ctx.attempt,
                "core_run_id": run_id, "bridge_instance_id": self._instance,
                "task_binding_ref": ctx.binding_ref}
        resp: httpx.Response | None = None
        for attempt in (1, 2):  # one retry on network error only
            try:
                with httpx.Client(transport=self._transport, timeout=self._timeout) as client:
                    resp = client.post(self._url, json=body, headers={
                        "Authorization": f"Bearer {self._token(ctx.tenant_id)}", "Idempotency-Key": ctx.command_key})
                break
            except httpx.TransportError:
                resp = None
        if resp is None:
            return self._unproven(ctx)
        if resp.status_code == 200:
            moved = self._store.transition(ctx.tenant_id, ctx.command_key, "binding_confirmed", core_run_id=run_id)
            if moved is None:
                current = self._store.get(ctx.tenant_id, ctx.command_key)
                if current is None or current.state != "binding_confirmed":
                    return BindResult(False, "pulso:binding_cas_lost")
            fact = STAGE_FACTS.get(ctx.stage)
            return BindResult(True, facts={name: name == fact for name in STAGE_FACTS.values()})
        if resp.status_code in (404, 409):
            return BindResult(False, "pulso:binding_failed")
        return self._unproven(ctx)

    def _unproven(self, ctx: Any) -> BindResult:
        """Timeout / 5xx: absence of effect is not proven -> receipt `manual_reconcile`, tools stay denied."""
        self._store.transition(ctx.tenant_id, ctx.command_key, "manual_reconcile", reason="binding_unproven")
        return BindResult(False, "pulso:binding_failed", effect_unproven=True)
