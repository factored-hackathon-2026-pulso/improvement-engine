"""Evaluate path on real Postgres 16: admission gate, replay, `fail` / `failed_infra`, zero operative writes.

Doubles: FakeBroker (the lab-broker is not part of L5), FixedBudgets, plus the engine stand-ins of world.py.
Real: PgRegistryStore, RegistryService, PulsoEvalPort, ScenarioEvaluator, PostgresStore eval DB, bridge tables."""

from __future__ import annotations

import threading
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta
from decimal import Decimal
from typing import Any

import psycopg
import pytest
from agent_core.domain import GatewayError, GatewayErrorKind
from agent_core.registry import PgRegistryStore, RegistryError, RegistryErrorCode
from agent_core.registry.models import Origin

from l5.world import (
    AGENT,
    bot_actor,
    common_kwargs,
    operative_counts,
    prompt_draft,
    seed_demo,
    suite_draft,
)
from pulso_core_runtime.evaluation.admission import (
    Admission,
    AdmissionDenied,
    InvocationContext,
    PgAdmissionStore,
    valid_context_ref,
)
from pulso_core_runtime.evaluation.budget import BudgetLimits
from pulso_core_runtime.evaluation.native import EvalComposition, EvaluationGate
from pulso_core_runtime.evaluation.report import PgReportStore
from pulso_core_runtime.registry_service import EvaluationRuntime

pytestmark = [pytest.mark.runtime, pytest.mark.pg]
NOW = datetime(2030, 1, 1, tzinfo=UTC)


@dataclass
class FakeBroker:
    allow: bool = True
    calls: list[dict[str, Any]] = field(default_factory=list)

    def check(self, *, tenant_id: str, binding_ref: str, scope: str, payload_digest: str) -> bool:
        self.calls.append({"tenant": tenant_id, "binding": binding_ref, "scope": scope})
        return self.allow


class FixedBudgets:
    def resolve(self, budget_ref: str, tenant_id: str) -> BudgetLimits | None:
        return None if budget_ref == "bud-missing" else BudgetLimits(budget_ref, Decimal("5"), 100000, 20)


class Counting:
    """Counts harness jobs (one `storage()` call per job)."""

    def __init__(self, inner: Any) -> None:
        self.inner, self.jobs = inner, 0

    def __call__(self) -> Any:
        self.jobs += 1
        return self.inner()


class World:
    def __init__(self, pg: Any, *, gateway: Any = None, now: datetime | None = None,
                 gate: EvaluationGate | None = None) -> None:
        self.pg = pg
        self.store = PgRegistryStore(lambda: psycopg.connect(pg.runtime, autocommit=False))
        seed_demo(self.store)
        kw = common_kwargs(pg.eval, gateway=gateway)
        self.storage = Counting(kw["storage"])
        kw["storage"] = self.storage
        self.broker = FakeBroker()
        self.clock_now = now or datetime.now(UTC)
        self.rt = EvaluationRuntime(
            store=self.store, composition=EvalComposition(**kw), clock=kw["clock"], ids=kw["ids"],
            admissions=PgAdmissionStore(pg.runtime), reports=PgReportStore(pg.runtime), broker=self.broker,
            budgets=FixedBudgets(), gate=gate, now=lambda: self.clock_now)
        self.actor = bot_actor()

    def frozen_proposal(self, suite: Any = None) -> tuple[str, str]:
        svc = self.rt.service
        p = svc.create_proposal(self.actor, AGENT, Origin.manual,
                                "change", idempotency_key="w-create")
        svc.put_draft(self.actor, p.proposal_id, [prompt_draft(), suite or suite_draft()], 0,
                      idempotency_key="w-put")
        svc.freeze(self.actor, p.proposal_id, idempotency_key="w-freeze")
        detail = svc.get_proposal(p.proposal_id)
        return p.proposal_id, detail.proposal.candidate_hash or ""

    def admit(self, pid: str, chash: str, ref: str = "ctx-1", *, attempt: int = 1, tenant: str = "t1",
              suite_version: str = "1.0.0", digest: str | None = None, deadline: datetime | None = None,
              budget_ref: str = "bud-1") -> Admission:
        digest = digest or self.rt._suite_digest(pid, "disputas-suite", suite_version) or "x"
        adm = Admission(ref, tenant, "job-1", "bind-1", pid, chash, "disputas-suite", suite_version, digest,
                        attempt, budget_ref, deadline or (self.clock_now + timedelta(hours=1)), "d" * 64)
        created, _ = self.rt.admissions.create(adm)
        return created

    def ctx(self, ref: str | None = "ctx-1", **over: Any) -> InvocationContext:
        base: dict[str, Any] = dict(tenant_id="t1", job_id="job-1", binding_ref="bind-1", binding_confirmed=True,
                                    evaluate_enabled=True, evaluation_context_ref=ref)
        return InvocationContext(**{**base, **over})

    def evaluate(self, pid: str, **ctx: Any) -> Any:
        return self.rt.evaluate(self.ctx(**ctx), self.actor, pid)


def test_pass_stores_full_report_and_touches_no_operative_table(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash)
    before = operative_counts(pg.runtime)
    out = w.evaluate(pid)
    after = operative_counts(pg.runtime)
    assert out.verdict == "pass" and out.eval_run_ref and not out.replayed
    allowed = {"reg_eval_runs", "reg_events", "reg_draft_writes"}  # what a native evaluate writes by design
    assert {t: n for t, n in after.items() if n != before[t] and t not in allowed} == {}
    assert after["reg_eval_runs"] == before["reg_eval_runs"] + 1
    assert after["runs"] == before["runs"] and after["audit_events"] == before["audit_events"]
    stored = w.rt.reports.get(pid, out.eval_run_ref)
    assert stored is not None and stored.report["verdict"] == "pass" and stored.report["items"]
    assert w.broker.calls == [{"tenant": "t1", "binding": "bind-1", "scope": "native_evaluate"}]
    with psycopg.connect(pg.eval) as conn:  # the evaluation runs went to the isolated eval DB
        assert conn.execute("SELECT count(*) FROM runs").fetchone()[0] >= 1  # type: ignore[index]


def test_same_admission_replays_without_a_second_run(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash)
    first = w.evaluate(pid)
    jobs = w.storage.jobs
    again = w.evaluate(pid)
    assert w.storage.jobs == jobs and again.replayed
    assert again.report_digest == first.report_digest and again.eval_run_ref == first.eval_run_ref


def test_fail_keeps_409_body_report_survives_and_replay_is_identical(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal(suite_draft(outcome="escalated"))
    w.admit(pid, chash)
    with pytest.raises(RegistryError) as first:
        w.evaluate(pid)
    assert first.value.code is RegistryErrorCode.gate_failed
    detail = w.rt.service.get_proposal(pid)
    assert detail.last_eval is None and detail.proposal.state.value == "draft"  # the upstream gap
    rows = w.rt.reports.list_for(pid)
    assert len(rows) == 1 and rows[0].gate_failed and rows[0].verdict == "fail"
    assert {k: v for k, v in rows[0].report.items() if k != "pulso_evidence"} == first.value.payload
    assert rows[0].report["pulso_evidence"]["closed_early"] is False  # bridge copy == 409 body + evidence
    jobs = w.storage.jobs
    with pytest.raises(RegistryError) as second:
        w.evaluate(pid)
    assert w.storage.jobs == jobs and second.value.payload == first.value.payload
    assert len(w.rt.reports.list_for(pid)) == 1


def test_failed_infra_is_never_pass_and_retry_needs_a_new_admission(pg) -> None:  # type: ignore[no-untyped-def]
    class Down:
        def generate(self, *a: Any, **k: Any) -> Any:
            raise GatewayError(GatewayErrorKind.unavailable)

    w = World(pg, gateway=Down())
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash, "ctx-1")
    out = w.evaluate(pid)
    assert out.verdict == "failed_infra" and out.report["verdict"] != "pass"
    assert w.rt.service.get_proposal(pid).proposal.state.value == "candidate"  # not evaluated, not reset
    jobs = w.storage.jobs
    replay = w.evaluate(pid)
    assert replay.verdict == "failed_infra" and replay.replayed and w.storage.jobs == jobs
    w.admit(pid, chash, "ctx-2", attempt=2)
    retry = w.evaluate(pid, evaluation_context_ref="ctx-2")
    assert retry.verdict == "failed_infra" and not retry.replayed and w.storage.jobs > jobs


def test_shared_service_fails_closed_without_admission(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, _ = w.frozen_proposal()
    report = w.rt.service.evaluate(w.actor, pid, "disputas-suite", "1.0.0", idempotency_key="direct-1")
    assert report.verdict == "failed_infra" and "no_admission" in report.detail
    assert w.storage.jobs == 0


CODES = {
    "missing": "admission_missing", "cross_tenant": "admission_cross_tenant",
    "wrong_proposal": "admission_proposal_mismatch", "candidate_changed": "candidate_changed",
    "suite_mismatch": "suite_mismatch", "expired": "admission_expired", "unknown": "evaluation_unknown",
    "disabled": "evaluate_disabled", "unconfirmed": "binding_not_confirmed", "broker_denied": "broker_denied",
    "bad_attempt": "admission_attempt_mismatch", "budget_unknown": "budget_unknown",
    "ref_invalid": "evaluation_context_invalid"}


@pytest.mark.parametrize("case", [
    "missing", "cross_tenant", "wrong_proposal", "candidate_changed", "suite_mismatch", "expired", "unknown",
    "disabled", "unconfirmed", "broker_denied", "bad_attempt", "budget_unknown", "ref_invalid"])
def test_admission_negatives_do_zero_harness_work(pg, case: str) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    pid, chash = w.frozen_proposal()
    ref = "ctx-1"
    kw: dict[str, Any] = {}
    match case:
        case "missing":
            ref = "ctx-none"
        case "cross_tenant":
            w.admit(pid, chash, tenant="t2")
        case "wrong_proposal":
            w.admit("p-other", chash, digest="d" * 64)
        case "candidate_changed":
            w.admit(pid, "0" * 64)
        case "suite_mismatch":
            w.admit(pid, chash, digest="f" * 64)
        case "expired":
            w.admit(pid, chash, deadline=w.clock_now - timedelta(seconds=1))
        case "unknown":
            w.admit(pid, chash)
            assert w.rt.admissions.transition("ctx-1", "admitted", "unknown")
        case "disabled":
            w.admit(pid, chash)
            kw["evaluate_enabled"] = False
        case "unconfirmed":
            w.admit(pid, chash)
            kw["binding_confirmed"] = False
        case "broker_denied":
            w.admit(pid, chash)
            w.broker.allow = False
        case "bad_attempt":
            w.admit(pid, chash)
            kw["evaluation_attempt"] = 9
        case "budget_unknown":
            w.admit(pid, chash, budget_ref="bud-missing")
        case "ref_invalid":
            ref = "x" * 201
    before = operative_counts(pg.runtime)
    with pytest.raises(AdmissionDenied) as denied:
        w.evaluate(pid, evaluation_context_ref=ref, **kw)
    assert denied.value.code == CODES[case]
    assert w.storage.jobs == 0
    assert operative_counts(pg.runtime) == before
    if case == "budget_unknown":  # proven no effect: the admission is usable again
        assert w.rt.admissions.get("ctx-1").state == "admitted"  # type: ignore[union-attr]


def test_tool_args_cannot_select_an_admission(pg) -> None:  # type: ignore[no-untyped-def]
    """`evaluate` takes `proposal_id` only; suite and admission come from the trusted context."""
    import inspect

    assert list(inspect.signature(World(pg).rt.evaluate).parameters) == ["ctx", "actor", "proposal_id", "audit"]


def test_concurrent_calls_with_one_admission_run_at_most_once(pg) -> None:  # type: ignore[no-untyped-def]
    """SV: two simultaneous evaluates with the same admission: CAS lets one run; the other is denied/replays."""
    w = World(pg)
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash)
    results: list[Any] = []

    def go() -> None:
        try:
            results.append(w.evaluate(pid))
        except (AdmissionDenied, RegistryError) as exc:
            results.append(exc)

    threads = [threading.Thread(target=go) for _ in range(2)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert w.storage.jobs == 1  # exactly one harness job (suite has one scenario, one repetition)
    assert sum(1 for r in results if getattr(r, "verdict", None) == "pass") >= 1


def test_context_ref_validation() -> None:
    assert valid_context_ref("abc_DEF-1.2:3") and valid_context_ref("a" * 200)
    for bad in ("", "a" * 201, "a b", "a\n", "ñ", "a/b", "a;b", None, 5):
        assert not valid_context_ref(bad)


def test_crash_after_consume_leaves_unknown_and_blocks_a_second_run(pg) -> None:  # type: ignore[no-untyped-def]
    """Simulated bridge death after the CAS and before a stored result: uncertain prior execution."""
    w = World(pg)
    pid, chash = w.frozen_proposal()
    w.admit(pid, chash)
    assert w.rt.admissions.transition("ctx-1", "admitted", "consumed")
    with pytest.raises(AdmissionDenied) as live:  # within the deadline it may still be running: no mutation
        w.evaluate(pid)
    assert live.value.code == "evaluation_in_progress"
    assert w.rt.admissions.get("ctx-1").state == "consumed"  # type: ignore[union-attr]
    w.clock_now += timedelta(hours=2)  # past the deadline no run can still be live: uncertain prior execution
    with pytest.raises(AdmissionDenied) as e:
        w.evaluate(pid)
    assert e.value.code == "evaluation_unknown" and w.storage.jobs == 0
    assert w.rt.admissions.get("ctx-1").state == "unknown"  # type: ignore[union-attr]
    with pytest.raises(AdmissionDenied):  # stays blocked: a retry needs evaluation_attempt+1
        w.evaluate(pid)
    w.admit(pid, chash, "ctx-2", attempt=2)
    assert w.evaluate(pid, evaluation_context_ref="ctx-2").verdict == "pass"
