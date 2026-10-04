"""M2a spend guard on real PG16 (PULSO_TEST_PG_ADMIN): ledger CHECK admits guard outcomes (l3_003), `meter_job_total`
semantics, and an end-to-end ceiling / kill-switch run through the real store."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from decimal import Decimal
from types import SimpleNamespace
from typing import Any

import psycopg
import pytest

from pulso_core_runtime.llm.guard import KillSwitch, SpendGuard, build_spend_guard, reconcile_ledger
from pulso_core_runtime.store.migrations import MIGRATIONS, apply_l3
from pulso_core_runtime.store.receipts import ReceiptStore

pytestmark = [pytest.mark.pg]
D = Decimal
T, J = "t1", "j1"


@pytest.fixture
def store(pg: Any) -> ReceiptStore:
    apply_l3(pg.runtime)
    return ReceiptStore(pg.runtime)


def _meter(store: ReceiptStore, stage: str, attempt: int, tenant: str = T, job: str = J, **kw: Any) -> None:
    store.meter_spend(tenant, job, stage, attempt, cost_usd=kw.pop("cost", "0"), cap_usd="1000", **kw)


@pytest.mark.parametrize("outcome", ["kill_switch", "ceiling_exceeded"])
def test_ledger_admits_guard_outcomes(store: ReceiptStore, outcome: str) -> None:
    store.ledger_record(T, J, "scout", 1, outcome=outcome, binding_ref="r")
    assert [r["outcome"] for r in store.ledger_rows(T, J, "scout", 1)] == [outcome]


def test_ledger_still_rejects_unknown_outcome(store: ReceiptStore) -> None:
    with pytest.raises(psycopg.errors.CheckViolation):
        store.ledger_record(T, J, "scout", 1, outcome="bogus", binding_ref="r")


def test_migrations_are_idempotent_and_ordered(pg: Any) -> None:
    apply_l3(pg.runtime)
    apply_l3(pg.runtime)
    with psycopg.connect(pg.runtime) as conn:
        names = [r[0] for r in conn.execute("SELECT name FROM pulso_bridge.migrations ORDER BY name")]
    assert names == sorted(n for n, _ in MIGRATIONS) and names[-1] == "l3_003_ledger_guard_outcomes"


def test_meter_job_total_empty_is_zero(store: ReceiptStore) -> None:
    assert store.meter_job_total(T, J) == D(0)


def test_meter_job_total_sums_cost_and_reserved_over_stages_and_attempts(store: ReceiptStore) -> None:
    _meter(store, "scout", 1, cost="0.10")
    _meter(store, "scout", 2, cost="0.20")
    _meter(store, "plan", 1, cost="0.05")
    assert store.meter_reserve(T, J, "plan", 1, amount="0.30", cap_usd="10")
    assert store.meter_reserve(T, J, "eval", 0, amount="0.07", cap_usd="10")  # reserved-only row
    assert store.meter_job_total(T, J) == D("0.72")


def test_meter_job_total_scopes_by_tenant_and_job(store: ReceiptStore) -> None:
    _meter(store, "scout", 1, cost="0.10")
    _meter(store, "scout", 1, cost="1", tenant="t2")
    _meter(store, "scout", 1, cost="2", job="j2")
    assert store.meter_job_total(T, J) == D("0.10")
    assert store.meter_job_total("t2", J) == D("1")
    assert store.meter_job_total(T, "j2") == D("2")
    assert store.meter_job_total("nobody", J) == D(0)


def test_meter_job_total_settle_moves_reserved_to_cost(store: ReceiptStore) -> None:
    assert store.meter_reserve(T, J, "scout", 1, amount="0.50", cap_usd="10")
    assert store.meter_job_total(T, J) == D("0.50")
    total, over = store.model_call_settle(T, J, "scout", 1, cap_usd="10", release_usd=D("0.50"), cost_usd=D("0.20"),
                                          tokens=15, usage_known=True, tokens_in=10, tokens_out=5, outcome="ok",
                                          binding_ref="r")
    assert (total, over) == (D("0.20"), False)
    assert store.meter_job_total(T, J) == D("0.20")  # reserved released, only the real cost remains


# --- end to end through the real store -----------------------------------------------------------------------

def _stack(store: ReceiptStore, guard: SpendGuard, tokens: tuple[int, int] = (100, 50), cost: str = "0.01"):
    from pulso_core_runtime.llm.metering import SpendMeteringGateway

    store.save_context("ref", T, J, "k", {"budget": {}}, datetime.now(UTC) + timedelta(hours=1))
    reported: list[dict[str, int]] = []

    class Inner:
        calls = 0

        def generate(self, *a: Any) -> Any:
            Inner.calls += 1
            reported.append({"tokens_in": tokens[0], "tokens_out": tokens[1]})
            return SimpleNamespace(tokens_in=tokens[0], tokens_out=tokens[1], cost_usd=cost, usage_known=True,
                                   model="m")

    ic = SimpleNamespace(tenant_id=T, job_id=J, stage="scout", attempt=1)
    gw = SpendMeteringGateway(Inner(), SimpleNamespace(lookup=lambda r: ic), store, lambda: "ref", guard=guard)
    return gw, Inner, reported


def test_ceiling_run_stops_and_ledgers_ceiling_exceeded(store: ReceiptStore) -> None:
    from agent_core.domain.errors import GatewayError, GatewayErrorKind

    guard = build_spend_guard(SimpleNamespace(ceiling_usd=D("0.025"), kill_file=None), store, env={})
    gw, Inner, reported = _stack(store, guard)
    for _ in range(3):
        gw.generate("p", {}, "es")
    with pytest.raises(GatewayError) as e:
        gw.generate("p", {}, "es")
    assert e.value.kind is GatewayErrorKind.refused and Inner.calls == 3
    rows = store.ledger_rows(T, J, "scout", 1)
    assert [r["outcome"] for r in rows] == ["ok"] * 3 + ["ceiling_exceeded"]
    assert store.meter_job_total(T, J) == D("0.03")
    assert reconcile_ledger([r for r in rows if r["outcome"] == "ok"], reported).ok


def test_kill_switch_ledgers_kill_switch_and_reconciles(store: ReceiptStore, tmp_path: Any) -> None:
    from agent_core.domain.errors import GatewayError

    marker = tmp_path / "kill"
    guard = SpendGuard(ceiling_usd=None, spent=lambda s: store.meter_job_total(*s), kill=KillSwitch(env={}, file=marker))
    gw, Inner, reported = _stack(store, guard)
    gw.generate("p", {}, "es")
    marker.write_text("1")
    for _ in range(2):
        with pytest.raises(GatewayError):
            gw.generate("p", {}, "es")
    rows = store.ledger_rows(T, J, "scout", 1)
    assert Inner.calls == 1 and [r["outcome"] for r in rows] == ["ok", "kill_switch", "kill_switch"]
    assert reconcile_ledger(rows, reported).ok  # refused rows carry 0 tokens
