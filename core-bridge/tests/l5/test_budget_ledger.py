"""EvalBudgetMeter charges the atomic capped ledger (`ReceiptStore.meter_spend`) on real PG16 (plan 17.3.5 Budget)."""

from __future__ import annotations

import threading
from decimal import Decimal

import psycopg
import pytest
from agent_core.ports import GenerationResult

from l5.test_arms import FakeArtifacts, req, runner
from l5.test_evaluate_path import World
from pulso_core_runtime.evaluation.arms import execution_id_for
from pulso_core_runtime.evaluation.budget import BudgetLimits, EvalBudgetMeter
from pulso_core_runtime.store.migrations import apply_l3 as ensure_schema
from pulso_core_runtime.store.receipts import ReceiptStore

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


def gen(cost: str) -> GenerationResult:
    return GenerationResult(output="x", tokens_in=1, tokens_out=1, cost_usd=Decimal(cost), model="m")


def ledger_row(dsn: str, tenant: str, job: str) -> tuple[Decimal, int]:
    with psycopg.connect(dsn) as conn:
        row = conn.execute("SELECT cost_usd, calls FROM pulso_bridge.budget_meter WHERE tenant_id=%s AND job_id=%s",
                           (tenant, job)).fetchone()
    assert row is not None
    return row[0], row[1]


def test_meter_charges_ledger_and_cap_refusal_never_overspends(pg) -> None:  # type: ignore[no-untyped-def]
    ensure_schema(pg.runtime)
    store = ReceiptStore(pg.runtime)
    meter = EvalBudgetMeter(BudgetLimits("b", cost_usd_max=Decimal("0.003")), ledger=store, tenant_id="t1",
                            job_id="job-a")
    for _ in range(3):
        meter.record(gen("0.001"))
    assert not meter.blocked() and ledger_row(pg.runtime, "t1", "job-a")[0] == Decimal("0.003")
    meter.record(gen("0.001"))  # the atomic statement refuses: ledger stays at the cap
    assert meter.blocked() and meter.exhausted == "cost_usd_max"
    assert ledger_row(pg.runtime, "t1", "job-a") == (Decimal("0.003"), 3)


def test_two_meters_one_job_cannot_exceed_the_cap_concurrently(pg) -> None:  # type: ignore[no-untyped-def]
    ensure_schema(pg.runtime)
    store = ReceiptStore(pg.runtime)
    limits = BudgetLimits("b", cost_usd_max=Decimal("0.005"))
    meters = [EvalBudgetMeter(limits, ledger=store, tenant_id="t1", job_id="job-c") for _ in range(2)]
    threads = [threading.Thread(target=lambda m=m: [m.record(gen("0.001")) for _ in range(5)])  # type: ignore[misc]
               for m in meters for _ in range(2)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    cost, calls = ledger_row(pg.runtime, "t1", "job-c")
    assert cost == Decimal("0.005") and calls == 5  # 20 attempts, exactly 5 accepted


def test_uncapped_budget_is_still_metered(pg) -> None:  # type: ignore[no-untyped-def]
    ensure_schema(pg.runtime)
    meter = EvalBudgetMeter(BudgetLimits("b"), ledger=ReceiptStore(pg.runtime), tenant_id="t1", job_id="job-u")
    meter.record(gen("0.25"))
    assert not meter.blocked() and ledger_row(pg.runtime, "t1", "job-u")[0] == Decimal("0.25")


def test_arm_runner_charges_the_ledger_and_a_tiny_cap_is_failed_infra(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    ensure_schema(pg.runtime)
    r = runner(w, artifacts=FakeArtifacts())
    r.ledger = ReceiptStore(pg.runtime)
    rep = r.run(req("k-ledger"), tenant_id="t1").report or {}
    assert rep["status"] == "completed"
    cost, calls = ledger_row(pg.runtime, "t1", execution_id_for("k-ledger", "t1"))
    assert cost == Decimal(rep["usage"]["cost_usd"]) and cost > 0 and calls >= 1

    class Tiny:
        def resolve(self, budget_ref: str, tenant_id: str) -> BudgetLimits:
            return BudgetLimits(budget_ref, Decimal("0.0001"))

    r.budgets = Tiny()
    bad = r.run(req("k-tiny"), tenant_id="t1").report or {}
    assert bad["status"] == "failed_infra" and "budget_exhausted" in bad["reason"]
    spent, _ = ledger_row(pg.runtime, "t1", execution_id_for("k-tiny", "t1")) if _row(pg.runtime, "k-tiny") else (0, 0)
    assert spent <= Decimal("0.0001")  # the refused spend was never applied


def _row(dsn: str, key: str) -> bool:
    with psycopg.connect(dsn) as conn:
        return conn.execute("SELECT 1 FROM pulso_bridge.budget_meter WHERE job_id=%s",
                            (execution_id_for(key, "t1"),)).fetchone() is not None
