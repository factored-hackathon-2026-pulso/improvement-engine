"""HTTP-campaign wrapper (`registry_http_contract` only; plan 17.3.5 mechanism A).

An `ApiExtension` placed BEFORE `registry_extension` registers `POST /v1/registry/proposals/{pid}/evaluate`;
Starlette matches the first-registered route, so this handler shadows upstream's. It reads
`X-Pulso-Evaluation-Context` (never logged), runs the SAME `AdmissionGate` through `EvaluationRuntime.evaluate`
and leaves upstream outcomes (409 `gate_failed` body, 429, `candidate_changed`) untouched. Missing/expired/
cross-tenant/hash or suite mismatch/uncertain prior execution -> 4xx problem+json before any harness work.
`single_handler_cleanup` (placed AFTER `registry_extension`) removes the shadowed upstream route so the route
table holds exactly one handler for the path. The header cannot protect the in-process Flow path."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from agent_core.domain import Principal, dumps
from agent_core.registry import RegistryError
from agent_core.registry.http import _Problem, problem_response
from fastapi import FastAPI, Request, Response

from pulso_core_runtime.evaluation.admission import AdmissionDenied, InvocationContext
from pulso_core_runtime.registry_service import EvaluationRuntime

HEADER = "X-Pulso-Evaluation-Context"
PATH = "/v1/registry/proposals/{pid}/evaluate"


def _denied(exc: AdmissionDenied) -> Response:
    body = {"type": f"urn:pulso:evaluation:{exc.code}", "title": exc.code, "status": exc.status, "code": exc.code}
    return _Problem(dumps(body), status_code=exc.status)


def default_tenant_of(actor: Principal) -> str | None:
    return actor.attrs.get("tenant")


def evaluation_admission_extension(runtime: EvaluationRuntime,
                                   who: Callable[[Request, str | None], Principal],
                                   tenant_of: Callable[[Principal], str | None] = default_tenant_of,
                                   ) -> Callable[[FastAPI, Any], None]:
    def install(app: FastAPI, authenticate: Any) -> None:
        @app.post(PATH)
        def evaluate(request: Request, pid: str, body: dict[str, Any]) -> Response:
            actor = who(request, request.headers.get("authorization"))
            ref = request.headers.get(HEADER)
            if ref is None:
                return _denied(AdmissionDenied("evaluation_context_missing", 403))
            adm = runtime.admissions.get(ref)
            if adm is None:
                return _denied(AdmissionDenied("admission_missing", 403))
            if tenant_of(actor) != adm.tenant_id:  # the tenant comes from the admission; the actor must match it
                return _denied(AdmissionDenied("admission_cross_tenant", 403))
            ctx = InvocationContext(tenant_id=adm.tenant_id, job_id=adm.job_id, binding_ref=adm.binding_ref,
                                    binding_confirmed=True, evaluate_enabled=True, evaluation_context_ref=ref,
                                    evaluation_attempt=adm.evaluation_attempt)
            # the campaign body names the suite; it must equal the admission (checked by the gate via digest)
            if (body.get("suite_id"), body.get("suite_version")) != (adm.suite_id, adm.suite_version):
                return _denied(AdmissionDenied("suite_mismatch", 409))
            try:
                outcome = runtime.evaluate(ctx, actor, pid)
            except AdmissionDenied as exc:
                return _denied(exc)
            except RegistryError as exc:
                return problem_response(request, exc)  # upstream outcome, unchanged
            return Response(dumps(outcome.report), media_type="application/json")

    return install


def single_handler_cleanup(app: FastAPI, authenticate: Any) -> None:
    seen: set[tuple[str, frozenset[str]]] = set()
    kept = []
    for route in app.router.routes:
        path, methods = getattr(route, "path", None), getattr(route, "methods", None)
        if path == PATH and methods:
            key = (path, frozenset(methods))
            if key in seen:
                continue  # shadowed upstream handler
            seen.add(key)
        kept.append(route)
    app.router.routes[:] = kept
