"""PulsoScenarioHarness / port behaviour without Postgres (in-memory eval storage)."""

from __future__ import annotations

import contextvars
import threading
import time
from decimal import Decimal
from typing import Any

import pytest
from agent_core.adapters.system_ids import SystemIds
from agent_core.domain import GatewayError, GatewayErrorKind
from agent_core.registry import (
    EvalRequest,
    EvalTarget,
    HarnessUnavailable,
    LocalSandbox,
    ScenarioEvaluator,
    SnapshotRegistry,
)
from agent_core.registry.evaluation.yardstick import Yardstick

from l5.world import common_kwargs, demo_pinned, suite_with
from pulso_core_runtime.evaluation.budget import BudgetLimits, EvalBudgetMeter
from pulso_core_runtime.evaluation.native import EvalComposition, EvaluationGate, PulsoEvalPort
from pulso_core_runtime.harness import ManifestEntry, PulsoScenarioHarness, run_key

pytestmark = pytest.mark.runtime


def _target(label: str = "candidate", registry: Any = None) -> EvalTarget:
    pinned = demo_pinned()
    return EvalTarget(label, pinned.release, registry or SnapshotRegistry(pinned.release, pinned.entities))  # type: ignore[arg-type]


def _run(harness: PulsoScenarioHarness, target: EvalTarget | None = None) -> Any:
    scenario = suite_with().scenarios[0]
    sandbox = LocalSandbox(SystemIds())
    t = target or _target()
    return harness.run(t, "atencion", scenario, sandbox.tools(sandbox.provision(scenario.seed, t)))


def test_refuses_a_non_snapshot_registry() -> None:
    """A live registry would resolve `alias=prod` to the production release."""

    class LivePostgresLike:
        def resolve_release(self, *a: Any) -> Any:
            raise AssertionError("must never be reached")

    with pytest.raises(HarnessUnavailable, match="target_registry_not_snapshot"):
        _run(PulsoScenarioHarness(mode="native", **common_kwargs()), _target(registry=LivePostgresLike()))


def test_native_run_records_distinct_keys_and_run_ids_in_memory() -> None:
    h = PulsoScenarioHarness(mode="native", **common_kwargs(), execution_id="ex-1")
    _run(h)
    _run(h)
    assert len({r.key for r in h.runs}) == 2 and [r.repetition for r in h.runs] == [0, 1]
    assert h.runs[0].key == run_key("ex-1", "resuelto", "candidate", 0, h._nonce)


def test_gateway_down_is_harness_unavailable() -> None:
    class Down:
        def generate(self, *a: Any, **k: Any) -> Any:
            raise GatewayError(GatewayErrorKind.unavailable)

    with pytest.raises(HarnessUnavailable, match="gateway_failed"):
        _run(PulsoScenarioHarness(mode="native", **common_kwargs(gateway=Down())))


def test_manifest_missing_is_harness_unavailable_in_task_modes() -> None:
    h = PulsoScenarioHarness(mode="task_builder", manifest={}, **common_kwargs())
    with pytest.raises(HarnessUnavailable, match="manifest_missing"):
        _run(h)
    with pytest.raises(ValueError):
        PulsoScenarioHarness(mode="task_builder", **common_kwargs())  # manifest is mandatory
    with pytest.raises(ValueError):
        PulsoScenarioHarness(mode="stateful_attention", manifest={}, **common_kwargs())  # needs binding ref


def test_principals_per_mode() -> None:
    scenario = suite_with().scenarios[0]
    entry = ManifestEntry(input={"k": "v"}, attrs={"stage": "evaluation"})
    tb = PulsoScenarioHarness(mode="task_builder", manifest={"resuelto": entry}, tenant_id="t1",
                              **common_kwargs())._principal(scenario, "session", entry)
    assert tb.type.value == "builder" and tb.roles == ["constructor"] and tb.id == "builder:pulso-constructor:t1"
    assert "actor" not in tb.attrs  # never a human actor
    st = PulsoScenarioHarness(mode="stateful_attention", manifest={}, evaluation_binding_ref="bind-9",
                              **common_kwargs())._principal(scenario, "session", None)
    assert st.type.value == "customer" and st.attrs["evaluation_binding_ref"] == "bind-9"
    nat = PulsoScenarioHarness(mode="native", **common_kwargs())._principal(scenario, "session", None)
    assert "evaluation_binding_ref" not in nat.attrs


def test_budget_jobs_deadline_and_cost() -> None:
    from datetime import UTC, datetime, timedelta

    meter = EvalBudgetMeter(BudgetLimits("b", jobs_max=1))
    h = PulsoScenarioHarness(mode="native", meter=meter, **common_kwargs())
    _run(h)
    with pytest.raises(HarnessUnavailable, match="budget_exhausted"):
        _run(h)
    expired = EvalBudgetMeter(BudgetLimits("b", deadline=datetime.now(UTC) - timedelta(seconds=1)))
    with pytest.raises(HarnessUnavailable, match="budget_exhausted"):
        _run(PulsoScenarioHarness(mode="native", meter=expired, **common_kwargs()))
    # cost: CitingGateway reports 0.002 USD per generation; a tiny cap is exceeded -> exhausted after the run
    cheap = EvalBudgetMeter(BudgetLimits("b", cost_usd_max=Decimal("0.0001")))
    with pytest.raises(HarnessUnavailable, match="budget_exhausted"):
        _run(PulsoScenarioHarness(mode="native", meter=cheap, **common_kwargs()))
    ok = EvalBudgetMeter(BudgetLimits("b", cost_usd_max=Decimal("1")))
    _run(PulsoScenarioHarness(mode="native", meter=ok, **common_kwargs()))
    usage = ok.usage()
    assert usage is not None and Decimal(usage["cost_usd"]) > 0 and ok.cost_known


def test_unknown_usage_reports_null() -> None:
    from agent_core.ports import GenerationResult

    meter = EvalBudgetMeter(BudgetLimits("b"))
    meter.record(GenerationResult(output="x", tokens_in=0, tokens_out=0, cost_usd=Decimal(0), model="m",
                                  usage_known=False))
    assert meter.usage() is None and not meter.cost_known


# --- contextvars through evaluator threads (SV) -----------------------------------------------------------

def test_contextvars_do_not_cross_into_evaluator_threads_so_context_is_explicit() -> None:
    """Fact test: `ScenarioEvaluator` runs jobs in a ThreadPoolExecutor; ContextVars set by the caller are NOT
    visible there. L5 therefore never relies on them: the admission/execution context is a constructor arg."""
    var: contextvars.ContextVar[str] = contextvars.ContextVar("pulso_eval_ctx", default="unset")
    seen: list[str] = []

    class Spy(PulsoScenarioHarness):
        def run(self, target: EvalTarget, agent_id: str, scenario: Any, tools: Any) -> Any:
            seen.append(var.get())
            return super().run(target, agent_id, scenario, tools)

    var.set("admission-123")
    h = Spy(mode="native", execution_id="explicit-ex", **common_kwargs())
    pinned = demo_pinned()
    cand = EvalTarget("candidate", pinned.release, SnapshotRegistry(pinned.release, pinned.entities))
    ScenarioEvaluator(h, LocalSandbox(SystemIds()), max_workers=1).run(
        EvalRequest(candidate=cand, new=Yardstick(metrics=[], suite=suite_with(1))))
    assert seen == ["unset"], "ContextVar leaked or pool behaviour changed: revisit the explicit-context design"
    assert h.runs and all(r.execution_id == "explicit-ex" for r in h.runs)


# --- gate / port ------------------------------------------------------------------------------------------

def _request() -> EvalRequest:
    pinned = demo_pinned()
    cand = EvalTarget("candidate", pinned.release, SnapshotRegistry(pinned.release, pinned.entities))
    return EvalRequest(candidate=cand, new=Yardstick(metrics=[], suite=suite_with(1)))


def test_port_without_admission_fails_closed() -> None:
    port = PulsoEvalPort(EvalComposition(**common_kwargs()), EvaluationGate())
    report = port.run(_request())
    assert report.verdict == "failed_infra" and "no_admission" in report.detail


def test_two_simultaneous_evaluations_serialise_and_the_overflow_is_evaluation_busy() -> None:
    gate = EvaluationGate(permits=1, max_wait_s=0.05, max_waiters=4)
    port = PulsoEvalPort(EvalComposition(**common_kwargs()), gate)
    entered, release = threading.Event(), threading.Event()

    def hold() -> None:
        with gate.slot():
            entered.set()
            release.wait(5)

    t = threading.Thread(target=hold)
    t.start()
    assert entered.wait(5)
    report, harness = port.run_admitted(_request(), execution_id="e2", budget=None)
    release.set()
    t.join()
    assert report.verdict == "failed_infra" and "evaluation_busy" in report.detail and harness is None
    ok, _ = port.run_admitted(_request(), execution_id="e3", budget=None)  # slot free again
    assert ok.verdict == "pass", ok.detail


def test_gate_never_runs_two_jobs_in_parallel() -> None:
    gate = EvaluationGate(permits=1, max_wait_s=5, max_waiters=8)
    active, peak = 0, 0
    lock = threading.Lock()

    def work() -> None:
        nonlocal active, peak
        with gate.slot():
            with lock:
                active += 1
                peak = max(peak, active)
            time.sleep(0.02)
            with lock:
                active -= 1

    threads = [threading.Thread(target=work) for _ in range(6)]
    [t.start() for t in threads]
    [t.join() for t in threads]
    assert peak == 1


def test_bounded_wait_queue_rejects_immediately_when_full() -> None:
    gate = EvaluationGate(permits=1, max_wait_s=5, max_waiters=0)
    with gate.slot():  # a free gate admits
        with pytest.raises(HarnessUnavailable, match="evaluation_busy"):  # no waiter room: immediate
            with gate.slot():
                pass
