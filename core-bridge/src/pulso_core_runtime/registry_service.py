"""L5 composition of the registry service and the evaluate path (plan 17.3.5, 17.4.1, 17.4.5).

* The process-wide shared `RegistryService` is built with a `PulsoEvalPort` that FAILS CLOSED
  (`failed_infra: no_admission`) unless an admission is bound.
* `EvaluationRuntime.evaluate` is the single evaluate entry point of the Flow path: it runs the
  `AdmissionGate`, then calls a per-admission `RegistryService` clone bound to `BoundEvaluator(admission)` with
  `idempotency_key="pulso-eval:"+evaluation_context_ref`, and persists the FULL report (stock `get_write`
  returns only the verdict; after `fail` the proposal returns to draft and `last_eval` is null).
* Same admission replays the stored report / stored `gate_failed` payload with no run and no quota."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any, Protocol

from agent_core.domain import Principal
from agent_core.registry import (
    DEFAULT_QUOTAS,
    EvalSuite,
    Quotas,
    RegistryError,
    RegistryErrorCode,
    RegistryService,
    RegistryStore,
)
from agent_core.registry.entities import content_hash
from agent_core.registry.models import AuditContext

from pulso_core_runtime.evaluation.admission import (
    Admission,
    AdmissionDenied,
    AdmissionGate,
    AdmissionStore,
    BrokerPort,
    InvocationContext,
    ProposalView,
    eval_key,
)
from pulso_core_runtime.evaluation.budget import BudgetLimits
from pulso_core_runtime.evaluation.native import BoundEvaluator, EvalComposition, EvaluationGate, PulsoEvalPort
from pulso_core_runtime.evaluation.report import ReportStore, StoredReport, digest_of

from agent_core.registry.validation import DEFAULT_LIMITS


class BudgetResolver(Protocol):
    def resolve(self, budget_ref: str, tenant_id: str) -> BudgetLimits | None: ...


@dataclass(frozen=True)
class EvaluateOutcome:
    verdict: str  # pass | failed_infra  (a `fail` raises RegistryError gate_failed, report stored)
    eval_run_ref: str | None
    report_digest: str
    replayed: bool
    report: dict[str, Any]


class EvaluationRuntime:
    def __init__(self, *, store: RegistryStore, composition: EvalComposition, clock: Any, ids: Any,
                 admissions: AdmissionStore, reports: ReportStore, broker: BrokerPort, budgets: BudgetResolver,
                 runs: Any = None, gate: EvaluationGate | None = None, limits: Any = DEFAULT_LIMITS,
                 quotas: Quotas = DEFAULT_QUOTAS, now: Callable[[], Any] | None = None) -> None:
        self._store, self._clock, self._ids = store, clock, ids
        self._runs, self._limits, self._quotas = runs, limits, quotas
        self.admissions, self.reports, self._budgets = admissions, reports, budgets
        self.port = PulsoEvalPort(composition, gate or EvaluationGate())
        kwargs: dict[str, Any] = {} if now is None else {"now": now}
        self.gate = AdmissionGate(admissions, broker, **kwargs)
        self.service = self._service(self.port)  # fail-closed shared service

    def _service(self, evaluator: Any) -> RegistryService:
        return RegistryService(self._store, evaluator, self._clock, self._ids, runs=self._runs,
                               limits=self._limits, quotas=self._quotas)

    # --- helpers ---------------------------------------------------------------------------------------

    def _suite_digest(self, proposal_id: str, suite_id: str, version: str) -> str | None:
        detail = self.service.get_proposal(proposal_id)
        for d in detail.changes:
            if d.kind == "eval_suite" and d.content.get("id") == suite_id and d.content.get("version") == version:
                return content_hash(EvalSuite.model_validate(d.content))
        try:
            return self.service.get_entity("eval_suite", suite_id, version).content_hash
        except RegistryError:
            return None

    def _stored_run_id(self, key: str) -> str | None:
        with self._store.transaction() as tx:
            write = tx.get_draft_write(key)
        return None if write is None else write.result_ref

    def _persist(self, proposal_id: str, ref: str, key: str, report: dict[str, Any], verdict: str,
                 gate_failed: bool) -> StoredReport:
        run_id = self._stored_run_id(key)
        return self.reports.put(proposal_id, run_id or f"synthetic:{ref}", run_id, verdict, gate_failed, report)

    # --- the Flow path ----------------------------------------------------------------------------------

    def evaluate(self, ctx: InvocationContext, actor: Principal, proposal_id: str, *,
                 audit: AuditContext | None = None) -> EvaluateOutcome:
        ref = ctx.evaluation_context_ref
        if ref is None:
            raise AdmissionDenied("evaluation_context_invalid", 422)
        key = eval_key(ref)  # validates charset/length before anything else
        adm_probe = self.admissions.get(ref)
        if adm_probe is None:
            raise AdmissionDenied("admission_missing", 403)
        detail = self.service.get_proposal(proposal_id)
        fresh = ProposalView(proposal_id, detail.proposal.state.value, detail.proposal.candidate_hash,
                             self._suite_digest(proposal_id, adm_probe.suite_id, adm_probe.suite_version))
        replay = self.service.get_write(key) is not None
        digest = digest_of({"proposal_id": proposal_id, "candidate_hash": adm_probe.candidate_hash,
                            "suite_id": adm_probe.suite_id, "suite_version": adm_probe.suite_version,
                            "suite_digest": adm_probe.suite_digest, "evaluation_context_ref": ref})
        adm: Admission = self.gate.begin(ctx, proposal_id, fresh, replay_exists=replay,
                                         suite_id=adm_probe.suite_id, suite_version=adm_probe.suite_version,
                                         native_payload_digest=digest)
        budget: BudgetLimits | None = None
        if not replay:
            budget = self._budgets.resolve(adm.budget_ref, adm.tenant_id)
            if budget is None:
                self.admissions.transition(ref, "consumed", "admitted")  # nothing happened: zero spend
                raise AdmissionDenied("budget_unknown", 403)
            budget = BudgetLimits(budget.budget_ref, budget.cost_usd_max, budget.tokens_max, budget.jobs_max,
                                  adm.deadline if budget.deadline is None else min(budget.deadline, adm.deadline))
        bound = BoundEvaluator(self.port, execution_id=ref, budget=budget, tenant_id=adm.tenant_id)
        clone = self._service(bound)
        try:
            report = clone.evaluate(actor, proposal_id, adm.suite_id, adm.suite_version,
                                    idempotency_key=key, audit=audit)
        except RegistryError as exc:
            if exc.code is RegistryErrorCode.gate_failed:
                stored = self._persist(proposal_id, ref, key, exc.payload, "fail", True)  # type: ignore[arg-type]
                exc.pulso_report_digest = stored.report_digest  # type: ignore[attr-defined]
                exc.pulso_eval_run_ref = stored.eval_run_id  # type: ignore[attr-defined]
                raise
            self._settle_after_error(ref, key, bound)
            raise
        except BaseException:
            self._settle_after_error(ref, key, bound)
            raise
        data = report.model_dump(mode="json")
        stored = self._persist(proposal_id, ref, key, data, report.verdict, False)
        return EvaluateOutcome(report.verdict, stored.eval_run_id, stored.report_digest, replay, data)

    def _settle_after_error(self, ref: str, key: str, bound: BoundEvaluator) -> None:
        if self._stored_run_id(key) is not None:
            return
        if bound.runs_started == 0:
            self.admissions.transition(ref, "consumed", "admitted")  # proven no effect: same admission reusable
        else:
            self.gate.mark_unknown(ref)  # a run began without a stored result: uncertain


def _conforms() -> None:  # pragma: no cover
    _ = AdmissionStore


def build_evaluation_runtime(ports: Any, *, runtime_dsn: str, eval_dsn: str, broker: BrokerPort,
                             budgets: BudgetResolver, gate: EvaluationGate | None = None,
                             check_isolation: bool = True) -> EvaluationRuntime:
    """Replacement for `build_registry_service_for_serve(ports)` (L2 `main._compose`):

        runtime = build_evaluation_runtime(ports, runtime_dsn=dsn, eval_dsn=eval_dsn, broker=..., budgets=...)
        service = runtime.service          # shared, fail-closed service for `build_api_deps(registry_service=...)`

    `ports` is Core's `ServePorts` (needs `registry_api`, `clock`, `ids`, `keys`, `gateway`, `providers`,
    `calibrations`, `transcript`, `classifier`, `uow_factory`). The harness uses its own `EvalAuthz`
    (synthetic principals only), never the live `ports.authz`."""
    from agent_core.composition.registry import UowRunReleases

    from pulso_core_runtime.evaluation.admission import PgAdmissionStore
    from pulso_core_runtime.evaluation.isolated_registry import assert_isolated
    from pulso_core_runtime.evaluation.report import PgReportStore
    from pulso_core_runtime.harness import EvalAuthz
    from agent_core.composition import EvalStorage

    api = ports.registry_api
    if api is None:
        raise ValueError("registry API is not enabled")
    if check_isolation:
        assert_isolated(runtime_dsn, eval_dsn)
    composition = EvalComposition(
        clock=ports.clock, ids=ports.ids, keys=ports.keys, gateway=ports.gateway,
        providers=lambda _scenario_id: ports.providers, calibrations=ports.calibrations, authz=EvalAuthz(),
        storage=lambda: EvalStorage(uow_factory=api.eval_uow_factory, audit=api.eval_audit,
                                    transcript=ports.transcript),
        classifier=ports.classifier)
    return EvaluationRuntime(store=api.store, composition=composition, clock=ports.clock, ids=ports.ids,
                             admissions=PgAdmissionStore(runtime_dsn), reports=PgReportStore(runtime_dsn),
                             broker=broker, budgets=budgets, runs=UowRunReleases(ports.uow_factory), gate=gate)


class FlowEvaluationGate:
    """Implements L3b's `tools.builder.EvaluationGate` protocol over `EvaluationRuntime.evaluate`.

    L3b has already enforced binding/commitment/broker (`native_evaluate`); this adds checks (iii)-(vi),(viii)
    and the per-admission service clone. `actor_for(ic)` yields the constructor bot principal."""

    def __init__(self, runtime: EvaluationRuntime, actor_for: Callable[[Any], Principal]) -> None:
        self._rt, self._actor_for = runtime, actor_for

    def evaluate(self, ic: Any, proposal_id: str, idempotency_key: str,
                 requested_suite: tuple[str | None, str | None]) -> tuple[Any, Any, str | None]:
        from agent_core.domain.shared import ToolStatus

        c = ic.commitment
        ref = ic.evaluation_context_ref
        if c is None or ref is None or idempotency_key != eval_key(ref):
            return ToolStatus.denied, None, "pulso:commitment_mismatch"
        ctx = InvocationContext(tenant_id=ic.tenant_id, job_id=ic.job_id, binding_ref=ic.binding_ref,
                                binding_confirmed=True, evaluate_enabled=bool(c.evaluate_enabled),
                                evaluation_context_ref=ref)
        try:
            out = self._rt.evaluate(ctx, self._actor_for(ic), proposal_id)
        except AdmissionDenied as exc:
            return ToolStatus.denied, None, f"pulso:{exc.code}"
        except RegistryError as exc:
            if exc.code is RegistryErrorCode.gate_failed:
                return (ToolStatus.error,
                        {"verdict": "fail", "eval_run_ref": getattr(exc, "pulso_eval_run_ref", None),
                         "report_digest": getattr(exc, "pulso_report_digest", None)}, "gate_failed")
            return ToolStatus.error, None, f"pulso:{exc.code.value}"
        return (ToolStatus.ok, {"verdict": out.verdict, "eval_run_ref": out.eval_run_ref,
                                "report_digest": out.report_digest}, None)
