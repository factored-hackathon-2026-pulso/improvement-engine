"""Native FIFO path (plan 17.3.5): `PulsoEvalPort` + process-wide evaluation gate.

`PulsoEvalPort.run` builds a fresh `ScenarioEvaluator(PulsoScenarioHarness(mode="native"), LocalSandbox(ids),
max_workers=1)` per call under a process-wide single-slot gate with a bounded wait queue. Waiting too long (or
too many waiters) -> `HarnessUnavailable("evaluation_busy")` -> `failed_infra`. Every failure becomes a
`failed_infra` report, never a pass and never an HTTP 500."""

from __future__ import annotations

import threading
from collections.abc import Callable
from contextlib import contextmanager
from dataclasses import dataclass
from typing import Any

from agent_core.registry import (
    EvalPort,
    EvalReport,
    EvalRequest,
    HarnessUnavailable,
    LocalSandbox,
    ScenarioEvaluator,
)

from pulso_core_runtime.evaluation.budget import BudgetLimits, EvalBudgetMeter
from pulso_core_runtime.harness import Mode, PulsoScenarioHarness


class EvaluationGate:
    """Process-wide `Semaphore(permits)` with a bounded number of waiters and a bounded wait."""

    def __init__(self, permits: int = 1, max_wait_s: float = 30.0, max_waiters: int = 8) -> None:
        self._sem = threading.Semaphore(permits)
        self._max_wait_s, self._max_waiters = max_wait_s, max_waiters
        self._waiters = 0
        self._lock = threading.Lock()
        self.in_flight = 0

    @contextmanager
    def slot(self) -> Any:
        got = self._sem.acquire(blocking=False)
        if not got:
            with self._lock:
                if self._waiters >= self._max_waiters:
                    raise HarnessUnavailable("evaluation_busy")
                self._waiters += 1
            try:
                got = self._sem.acquire(timeout=self._max_wait_s)
            finally:
                with self._lock:
                    self._waiters -= 1
        if not got:
            raise HarnessUnavailable("evaluation_busy")
        with self._lock:
            self.in_flight += 1
        try:
            yield
        finally:
            with self._lock:
                self.in_flight -= 1
            self._sem.release()


@dataclass(frozen=True)
class EvalComposition:
    """Everything a harness needs except the per-evaluation context (explicit, no ContextVar)."""

    clock: Any
    ids: Any
    keys: Any
    gateway: Any
    providers: Callable[[str], Any]
    calibrations: Any
    authz: Any
    storage: Callable[[], Any]
    classifier: Any = None
    config: Any = None

    def harness(self, mode: Mode, **extra: Any) -> PulsoScenarioHarness:
        return PulsoScenarioHarness(
            mode=mode, clock=self.clock, ids=self.ids, keys=self.keys, gateway=self.gateway,
            providers=self.providers, calibrations=self.calibrations, authz=self.authz, storage=self.storage,
            classifier=self.classifier, config=self.config, **extra)


def infra_report(detail: str) -> EvalReport:
    return EvalReport(verdict="failed_infra", detail=detail[:200])


class PulsoEvalPort:
    """Shared-service port. With no admission bound it fails closed (`failed_infra`, `no_admission`)."""

    def __init__(self, composition: EvalComposition, gate: EvaluationGate) -> None:
        self._comp, self._gate = composition, gate

    def run(self, request: EvalRequest) -> EvalReport:
        return infra_report("HarnessUnavailable: no_admission")

    def run_admitted(self, request: EvalRequest, *, execution_id: str, budget: BudgetLimits | None,
                     tenant_id: str = "pulso") -> tuple[EvalReport, PulsoScenarioHarness | None]:
        meter = EvalBudgetMeter(budget) if budget is not None else None
        try:
            with self._gate.slot():
                harness = self._comp.harness("native", execution_id=execution_id, meter=meter,
                                             tenant_id=tenant_id)
                evaluator = ScenarioEvaluator(harness, LocalSandbox(self._comp.ids), max_workers=1)
                return evaluator.run(request), harness
        except HarnessUnavailable as exc:
            return infra_report(f"HarnessUnavailable: {exc}"), None
        except Exception as exc:  # unexpected: fail closed without leaking the message
            return infra_report(f"unexpected: {type(exc).__name__}"), None


class BoundEvaluator:
    """`EvalPort` bound to one admission: the per-admission `RegistryService` clone uses this."""

    def __init__(self, port: PulsoEvalPort, *, execution_id: str, budget: BudgetLimits | None,
                 tenant_id: str) -> None:
        self._port, self._execution_id, self._budget, self._tenant = port, execution_id, budget, tenant_id
        self.harness: PulsoScenarioHarness | None = None
        self.runs_started = 0

    def run(self, request: EvalRequest) -> EvalReport:
        self.runs_started += 1
        report, self.harness = self._port.run_admitted(
            request, execution_id=self._execution_id, budget=self._budget, tenant_id=self._tenant)
        return report


def _conforms(x: PulsoEvalPort, y: BoundEvaluator) -> tuple[EvalPort, EvalPort]:
    return x, y
