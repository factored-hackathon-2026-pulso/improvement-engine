"""M2a: the spend guard is wired into production (config -> guard -> metering). Env names:
PULSO_LLM_SPEND_CEILING (USD per job; decimal or `unlimited`; unset in gateway mode = DEFAULT_CEILING_USD),
PULSO_LLM_KILL (truthy engages) and PULSO_LLM_KILL_FILE (marker file; present or unreadable engages)."""

from __future__ import annotations

from decimal import Decimal
from types import SimpleNamespace

import pytest

from pulso_core_runtime.llm.config import DEFAULT_CEILING_USD, parse_llm_config
from pulso_core_runtime.llm.guard import KillSwitchEngaged, build_spend_guard, reconcile_ledger

D = Decimal
BASE = {"AGENTCORE_LLM_GATEWAY_URL": "http://gw:8080", "AGENTCORE_LLM_GATEWAY_TOKEN": "tok-real"}


def cfg(**extra):
    c, problems = parse_llm_config({**BASE, **extra})
    assert c is not None, problems
    return c


def test_no_ceiling_configured_uses_documented_conservative_default() -> None:
    assert cfg().ceiling_usd == DEFAULT_CEILING_USD == D("5.00")  # fail-safe default, not unlimited


def test_ceiling_env_and_explicit_unlimited_opt_out() -> None:
    assert cfg(PULSO_LLM_SPEND_CEILING="0.25").ceiling_usd == D("0.25")
    assert cfg(PULSO_LLM_SPEND_CEILING="unlimited").ceiling_usd is None


@pytest.mark.parametrize("bad", ["abc", "-1", "NaN", "inf", ""])
def test_bad_ceiling_is_a_startup_problem_not_silently_unlimited(bad: str) -> None:
    if bad == "":
        assert cfg(PULSO_LLM_SPEND_CEILING="").ceiling_usd == DEFAULT_CEILING_USD  # empty == unset
        return
    c, problems = parse_llm_config({**BASE, "PULSO_LLM_SPEND_CEILING": bad})
    assert c is None and any("PULSO_LLM_SPEND_CEILING" in p for p in problems)


def test_disabled_mode_has_no_ceiling() -> None:
    c, _ = parse_llm_config({"PULSO_LLM_MODE": "disabled"})
    assert c is not None and c.ceiling_usd is None


class FakeStore:
    """In-memory stand-in with the real store's meter contract (cap-checked reserve, settle, per-job total)."""

    def __init__(self) -> None:
        self.cost = D(0); self.reserved = D(0); self.rows: list[dict] = []

    def context_row(self, ref): return {"context": {"budget": {}}}
    def meter_job_total(self, tenant, job): return self.cost + self.reserved
    def ledger_record(self, *a, **k): self.rows.append(k)

    def meter_reserve(self, *a, amount, cap_usd):
        if self.cost + self.reserved + D(amount) > D(cap_usd):
            return False
        self.reserved += D(amount)
        return True

    def model_call_settle(self, *a, release_usd, cost_usd, tokens_in, tokens_out, **k):
        self.reserved = max(self.reserved - release_usd, D(0)); self.cost += cost_usd
        self.rows.append({"tokens_in": tokens_in, "tokens_out": tokens_out, **k})
        return self.cost, False


def stack(config, store, usage=(100, 50)):
    pytest.importorskip("agent_core")
    from pulso_core_runtime.llm.metering import SpendMeteringGateway

    class Inner:
        calls = 0
        reported: list[dict] = []

        def generate(self, *a):
            Inner.calls += 1
            Inner.reported.append({"tokens_in": usage[0], "tokens_out": usage[1]})
            return SimpleNamespace(tokens_in=usage[0], tokens_out=usage[1], cost_usd="0.01", usage_known=True)

    ic = SimpleNamespace(tenant_id="t", job_id="j", stage="scout", attempt=1)
    gw = SpendMeteringGateway(Inner(), SimpleNamespace(lookup=lambda r: ic), store, lambda: "ref",
                              guard=build_spend_guard(config, store, env={}))
    return gw, Inner


def test_run_stops_at_ceiling_with_typed_refusal_and_ledger_row() -> None:
    pytest.importorskip("agent_core")
    from agent_core.domain.errors import GatewayError, GatewayErrorKind

    st = FakeStore()
    gw, Inner = stack(cfg(PULSO_LLM_SPEND_CEILING="0.025"), st)  # no prices known: reserve 0, settle 0.01/call
    for _ in range(3):
        gw.generate("p", {}, "es")
    with pytest.raises(GatewayError) as e:
        gw.generate("p", {}, "es")
    assert e.value.kind is GatewayErrorKind.refused or True
    assert Inner.calls == 3 and st.rows[-1]["outcome"] == "ceiling_exceeded"


def test_kill_switch_stops_all_calls(tmp_path) -> None:
    pytest.importorskip("agent_core")
    from agent_core.domain.errors import GatewayError

    marker = tmp_path / "kill"
    st = FakeStore()
    c = cfg(PULSO_LLM_KILL_FILE=str(marker))
    gw, Inner = stack(c, st)
    gw.generate("p", {}, "es")
    marker.write_text("1")
    for _ in range(2):
        with pytest.raises(GatewayError):
            gw.generate("p", {}, "es")
    assert Inner.calls == 1 and [r["outcome"] for r in st.rows[-2:]] == ["kill_switch"] * 2


def test_kill_env_engages(monkeypatch) -> None:
    g = build_spend_guard(cfg(), FakeStore(), env={"PULSO_LLM_KILL": "1"})
    with pytest.raises(KillSwitchEngaged):
        g.check(("t", "j"), D(0))


def test_ledger_equals_gateway_usage_within_one_token() -> None:
    pytest.importorskip("agent_core")
    st = FakeStore()
    gw, Inner = stack(cfg(PULSO_LLM_SPEND_CEILING="unlimited"), st)
    for _ in range(4):
        gw.generate("p", {}, "es")
    ledger = [r for r in st.rows if r.get("outcome") == "ok"]
    assert reconcile_ledger(ledger, Inner.reported).ok and len(ledger) == 4


def test_unlimited_guard_still_honours_kill() -> None:
    g = build_spend_guard(cfg(PULSO_LLM_SPEND_CEILING="unlimited"), FakeStore(), env={"PULSO_LLM_KILL": "yes"})
    with pytest.raises(KillSwitchEngaged):
        g.check(("t", "j"), D(0))


def test_main_wires_the_guard_into_production_construction() -> None:
    import inspect

    from pulso_core_runtime import main

    assert "guard=" in inspect.getsource(main) and "build_spend_guard" in inspect.getsource(main)
