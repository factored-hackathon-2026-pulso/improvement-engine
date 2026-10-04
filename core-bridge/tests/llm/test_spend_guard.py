"""M2a RED: run-level spend ceiling, kill switch and ledger-vs-gateway reconciliation on the single Python ledger."""

from __future__ import annotations

from decimal import Decimal

import pytest

from pulso_core_runtime.llm.guard import (
    CeilingExceeded, KillSwitch, KillSwitchEngaged, SpendGuard, reconcile_ledger,
)

D = Decimal


def test_ceiling_refuses_call_that_would_exceed_cap() -> None:
    g = SpendGuard(ceiling_usd=D("1.00"), spent=lambda scope: D("0.90"))
    g.check(("t", "j"), D("0.10"))  # exactly at the cap is allowed
    with pytest.raises(CeilingExceeded):
        g.check(("t", "j"), D("0.100001"))


def test_ceiling_none_means_unlimited() -> None:
    SpendGuard(ceiling_usd=None, spent=lambda s: D("999")).check(("t", "j"), D("1"))


def test_ceiling_rejects_bad_values() -> None:
    with pytest.raises(ValueError):
        SpendGuard(ceiling_usd=D("-1"), spent=lambda s: D(0))
    with pytest.raises(ValueError):
        SpendGuard(ceiling_usd=D("NaN"), spent=lambda s: D(0))


def test_kill_switch_env_and_file(tmp_path) -> None:
    f = tmp_path / "kill"
    ks = KillSwitch(env={}, file=f)
    ks.check()
    f.write_text("x")
    with pytest.raises(KillSwitchEngaged):
        ks.check()
    f.unlink()
    with pytest.raises(KillSwitchEngaged):
        KillSwitch(env={"PULSO_LLM_KILL": "1"}, file=f).check()
    KillSwitch(env={"PULSO_LLM_KILL": "0"}, file=f).check()


def test_guard_checks_kill_switch_first() -> None:
    g = SpendGuard(ceiling_usd=D("1"), spent=lambda s: D(0), kill=KillSwitch(env={"PULSO_LLM_KILL": "true"}))
    with pytest.raises(KillSwitchEngaged):
        g.check(("t", "j"), D(0))


def test_reconcile_within_one_token() -> None:
    led = [{"tokens_in": 10, "tokens_out": 5}, {"tokens_in": 3, "tokens_out": 2}]
    gw = [{"tokens_in": 10, "tokens_out": 6}, {"tokens_in": 3, "tokens_out": 2}]
    r = reconcile_ledger(led, gw)
    assert r.ok and r.ledger_tokens == 20 and r.gateway_tokens == 21 and r.delta == 1


def test_reconcile_flags_drift_over_one_token() -> None:
    r = reconcile_ledger([{"tokens_in": 10, "tokens_out": 5}], [{"tokens_in": 10, "tokens_out": 8}])
    assert not r.ok and r.delta == 3


def test_reconcile_ignores_unknown_usage_rows_but_counts_them() -> None:
    r = reconcile_ledger([{"tokens_in": 4, "tokens_out": 1}, {"tokens_in": 0, "tokens_out": 0, "usage_known": False}],
                         [{"tokens_in": 4, "tokens_out": 1}])
    assert r.ok and r.unknown_rows == 1


def _metering_with_guard(guard):
    pytest.importorskip("agent_core")
    from types import SimpleNamespace

    from pulso_core_runtime.llm.metering import SpendMeteringGateway

    class Store:
        def __init__(self): self.rows = []
        def context_row(self, ref): return {"context": {"budget": {}}}
        def ledger_record(self, *a, **k): self.rows.append(k)
        def meter_reserve(self, *a, **k): return True
        def model_call_settle(self, *a, **k): return (D(0), False)

    class Inner:
        called = 0
        def generate(self, *a): Inner.called += 1; return SimpleNamespace(tokens_in=1, tokens_out=1, cost_usd="0", usage_known=True)

    ic = SimpleNamespace(tenant_id="t", job_id="j", stage="scout", attempt=1)
    st = Store()
    gw = SpendMeteringGateway(Inner(), SimpleNamespace(lookup=lambda r: ic), st, lambda: "ref", guard=guard)
    return gw, st, Inner


def test_metering_refuses_on_ceiling_and_kill_and_logs_ledger() -> None:
    pytest.importorskip("agent_core")
    from agent_core.domain.errors import GatewayError

    for guard, outcome in ((SpendGuard(ceiling_usd=D("0"), spent=lambda s: D("1")), "ceiling_exceeded"),
                           (SpendGuard(ceiling_usd=None, spent=lambda s: D(0),
                                       kill=KillSwitch(env={"PULSO_LLM_KILL": "1"})), "kill_switch")):
        gw, st, Inner = _metering_with_guard(guard)
        with pytest.raises(GatewayError):
            gw.generate("p", {}, "es")
        assert st.rows[-1]["outcome"] == outcome and Inner.called == 0


def test_kill_switch_fails_closed_when_file_unreadable(tmp_path, monkeypatch) -> None:
    import os

    f = tmp_path / "kill"
    real = os.stat

    def boom(p, *a, **k):
        if str(p) == str(f):
            raise PermissionError("denied")
        return real(p, *a, **k)

    monkeypatch.setattr(os, "stat", boom)
    with pytest.raises(KillSwitchEngaged):
        KillSwitch(env={}, file=f).check()


def test_guard_fails_closed_on_non_finite_spend_or_reserve() -> None:
    with pytest.raises(CeilingExceeded):
        SpendGuard(ceiling_usd=D("1"), spent=lambda s: D("NaN")).check(("t", "j"), D("0"))
    with pytest.raises(CeilingExceeded):
        SpendGuard(ceiling_usd=D("1"), spent=lambda s: D("0")).check(("t", "j"), D("-5"))


def test_check_and_reserve_are_serialised_per_guard() -> None:
    import threading
    import time

    pytest.importorskip("agent_core")
    spent = [D(0)]
    guard = SpendGuard(ceiling_usd=D("0.5"), spent=lambda s: spent[0])
    gw, st, Inner = _metering_with_guard(guard)

    def slow_reserve(*a, **k):
        time.sleep(0.05)
        spent[0] += D("1")
        return True

    st.meter_reserve = slow_reserve
    res = []

    def run():
        try:
            gw.generate("p", {}, "es")
            res.append("ok")
        except Exception:  # noqa: BLE001
            res.append("refused")

    ts = [threading.Thread(target=run) for _ in range(4)]
    [t.start() for t in ts]
    [t.join() for t in ts]
    assert res.count("ok") == 1
