"""`register()`-style router module for `/internal/v1/evaluation/*` (L3a owns `internal/app.py`).

Wiring line for `build_internal_app(..., handlers=...)`:

    from pulso_core_runtime.evaluation.routes import register
    register(handlers, evaluation_deps)

`handlers` maps `"METHOD path"` (the `ROUTES` table paths) to `handler(request, claims)`. Auth (service JWT,
audience, purpose, jti) is already enforced by `app.py` before a handler runs. Errors use the D.1 envelope."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

import pydantic
from fastapi import Request
from fastapi.responses import JSONResponse
from pydantic import BaseModel, ConfigDict, Field, field_validator
from starlette.concurrency import run_in_threadpool

from pulso_core_runtime.evaluation.admission import (
    Admission,
    AdmissionDenied,
    BrokerPort,
    ProposalView,
    derive_context_ref,
    valid_context_ref,
)
from pulso_core_runtime.evaluation.arms import ArmDenied, ArmRunner
from pulso_core_runtime.evaluation.report import digest_of
from pulso_core_runtime.internal.auth import Claims
from pulso_core_runtime.timefmt import parse_z_timestamp
from pulso_core_runtime.registry_service import BudgetResolver, EvaluationRuntime


class AdmissionRequest(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    schema_version: str = "1"
    evaluation_context_ref: str | None = None  # deprecated: derived server-side; accepted only when equal
    binding_ref: str = Field(min_length=1, max_length=200)
    proposal_id: str = Field(min_length=1, max_length=200)
    candidate_hash: str = Field(min_length=1, max_length=128)
    suite_id: str = Field(min_length=1, max_length=200)
    suite_version: str = Field(min_length=1, max_length=64)
    suite_digest: str = Field(min_length=1, max_length=128)
    evaluation_attempt: int = Field(ge=1)
    budget_ref: str = Field(min_length=1, max_length=200)
    deadline: str
    request_digest: str = Field(pattern=r"^[0-9a-f]{64}$")

    @field_validator("deadline")
    @classmethod
    def _deadline_is_z(cls, v: str) -> str:
        parse_z_timestamp(v)
        return v


@dataclass
class EvaluationDeps:
    runtime: EvaluationRuntime
    arms: ArmRunner
    broker: BrokerPort
    budgets: BudgetResolver
    now: Callable[[], datetime] = lambda: datetime.now(UTC)


def _err(code: str, status: int, *, retryable: bool = False, details: dict[str, Any] | None = None) -> JSONResponse:
    from pulso_core_runtime.internal.app import envelope

    return envelope(code, retryable=retryable, details=details, status=status)


def _tenant(claims: Claims) -> str:
    return claims.tenant_id or ""


def build_handlers(deps: EvaluationDeps) -> dict[str, Callable[[Request, Claims], Any]]:
    async def admit(request: Request, claims: Claims) -> JSONResponse:
        tenant = _tenant(claims)
        try:
            body = AdmissionRequest.model_validate(await request.json())
        except (pydantic.ValidationError, ValueError):
            return _err("pulso:invalid_request", 422)
        key = request.headers.get("idempotency-key")
        if key is not None and not valid_context_ref(key):
            return _err("pulso:invalid_request", 422, details={"fields": ["Idempotency-Key"]})
        job_id = str(claims.raw.get("job_id", ""))
        if not tenant or not job_id:
            return _err("pulso:auth_denied", 403)
        try:
            derived = derive_context_ref(tenant, job_id, body.binding_ref, body.proposal_id, body.candidate_hash,
                                         body.evaluation_attempt)
        except ValueError:
            return _err("pulso:invalid_request", 422)  # a `|` in a field would let two admissions share one ref
        if body.evaluation_context_ref is not None and body.evaluation_context_ref != derived:
            return _err("pulso:evaluation_context_invalid", 422)  # client-chosen refs are not annex D.4
        body = body.model_copy(update={"evaluation_context_ref": derived})
        try:
            return await run_in_threadpool(_admit_sync, deps, body, tenant, job_id)
        except AdmissionDenied as exc:
            return _err(f"pulso:{exc.code}", exc.status)

    async def arm_run(request: Request, claims: Claims) -> JSONResponse:
        try:
            raw = await request.json()
        except ValueError:
            return _err("pulso:invalid_request", 422)
        header = request.headers.get("idempotency-key")
        if header is not None:  # annex D.4: the key travels in the header; a body key must agree
            if not valid_context_ref(header) or not isinstance(raw, dict) or (
                    raw.get("idempotency_key") is not None and raw["idempotency_key"] != header):
                return _err("pulso:invalid_request", 422, details={"fields": ["Idempotency-Key"]})
            raw = {**raw, "idempotency_key": header}
        try:
            row = await run_in_threadpool(deps.arms.run, raw, tenant_id=_tenant(claims))
        except ArmDenied as exc:
            return _err(f"pulso:{exc.code}", exc.status)
        return JSONResponse(row.report or {"execution_id": row.execution_id, "status": row.status})

    async def arm_read(request: Request, claims: Claims) -> JSONResponse:
        params = request.path_params
        tenant = _tenant(claims)
        row = (deps.arms.read_by_key(params["key"], tenant_id=tenant) if "key" in params
               else deps.arms.read(params["arm_id"], tenant_id=tenant))
        if row is None:
            return _err("pulso:not_found", 404)
        return JSONResponse(row.report or {"execution_id": row.execution_id, "status": row.status})

    return {
        "POST /evaluation/admissions": admit,
        "POST /evaluation/arms/{arm_id}/run": arm_run,
        "POST /evaluation/arms/run": arm_run,
        "GET /evaluation/arms/{arm_id}": arm_read,
        "GET /evaluation/arms/by-key/{key}": arm_read,
    }


def _admit_sync(deps: EvaluationDeps, body: AdmissionRequest, tenant: str, job_id: str) -> JSONResponse:
    rt = deps.runtime
    try:
        allowed = deps.broker.check(tenant_id=tenant, binding_ref=body.binding_ref, scope="evaluation_admit",
                                    payload_digest=digest_of(body.model_dump(mode="json", exclude={"request_digest"})))
    except Exception:
        allowed = False
    if not allowed:
        raise AdmissionDenied("broker_denied", 403)
    if parse_z_timestamp(body.deadline) <= deps.now():
        raise AdmissionDenied("admission_expired", 409)
    if deps.budgets.resolve(body.budget_ref, tenant) is None:
        raise AdmissionDenied("budget_unknown", 403)
    try:
        detail = rt.service.get_proposal(body.proposal_id)
    except Exception:
        raise AdmissionDenied("proposal_not_found", 404) from None
    fresh = ProposalView(body.proposal_id, detail.proposal.state.value, detail.proposal.candidate_hash,
                         rt._suite_digest(body.proposal_id, body.suite_id, body.suite_version))
    if fresh.state != "candidate" or fresh.candidate_hash != body.candidate_hash:
        raise AdmissionDenied("candidate_changed", 409)
    if fresh.suite_digest != body.suite_digest:
        raise AdmissionDenied("suite_mismatch", 409)
    adm = Admission(str(body.evaluation_context_ref), tenant, job_id, body.binding_ref, body.proposal_id,
                    body.candidate_hash, body.suite_id, body.suite_version, body.suite_digest,
                    body.evaluation_attempt, body.budget_ref, parse_z_timestamp(body.deadline), body.request_digest)
    stored, created = rt.admissions.create(adm)
    if not created and (stored.request_digest, stored.tenant_id, stored.job_id, stored.binding_ref,
                        stored.proposal_id, stored.candidate_hash, stored.suite_id, stored.suite_version,
                        stored.suite_digest, stored.evaluation_attempt, stored.budget_ref) != (
            adm.request_digest, tenant, adm.job_id, adm.binding_ref, adm.proposal_id, adm.candidate_hash,
            adm.suite_id, adm.suite_version, adm.suite_digest, adm.evaluation_attempt, adm.budget_ref):
        raise AdmissionDenied("idempotency_conflict", 409)  # a replay must be the very same admission
    return JSONResponse({"schema_version": "1", "evaluation_context_ref": stored.evaluation_context_ref,
                         "state": stored.state}, status_code=201 if created else 200)


def register(handlers: dict[str, Callable[[Request, Claims], Any]], deps: EvaluationDeps) -> None:
    """Adds the evaluation handlers to the dict given to `build_internal_app(handlers=...)`."""
    handlers.update(build_handlers(deps))
