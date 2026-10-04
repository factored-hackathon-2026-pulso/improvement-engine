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
