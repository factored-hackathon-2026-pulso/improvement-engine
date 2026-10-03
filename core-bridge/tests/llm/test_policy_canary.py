"""Light canary: model alias and price come from OUR runtime config (stage pin), never from the stage agent, the
prompt or the registry profile the caller resolved; cost is computed from our table."""

from __future__ import annotations

from decimal import Decimal
from types import SimpleNamespace

import pytest
from agent_core.domain.errors import GatewayError
from pulso_core_runtime.llm.policy import ModelPolicy, StageModelPolicy

from .test_metering_v2 import (  # noqa: F401
    A,
    Inner,
    J,
    S,
    T,
    call,
    gateway,
    ledger,
    result,
    store,
)

pytestmark = [pytest.mark.pg]
POLICY = ModelPolicy({S: StageModelPolicy("pulso-scout-llm", "pinned-model", Decimal("0.10"), Decimal("0.32"), 1000)})


def _profile(**over: object) -> SimpleNamespace:
    base = {"endpoint_alias": "pulso-scout-llm", "model": "pinned-model", "max_tokens": 1000,
            "price": SimpleNamespace(input_per_mtok=Decimal("0.10"), output_per_mtok=Decimal("0.32"))}
    return SimpleNamespace(**{**base, **over})


@pytest.mark.parametrize("over", [
    {"model": "expensive-model"}, {"endpoint_alias": "other-alias"}, {"max_tokens": 1_000_000},
    {"price": SimpleNamespace(input_per_mtok=Decimal(1000000), output_per_mtok=Decimal(1000000))}])
def test_alias_model_or_price_not_in_our_pin_never_reaches_the_gateway(store, over) -> None:  # noqa: F811
    inner = Inner(result())
    with pytest.raises(GatewayError):
        call(gateway(store, inner, policy=POLICY, profile=_profile(**over)))
    assert inner.calls == 0
    assert [r["outcome"] for r in ledger(store)] == ["policy_denied"]


def test_cost_is_computed_from_our_price_table_not_the_reported_one(store) -> None:  # noqa: F811
    reported = result(1000, 500, cost="1500.000000")  # a gateway/price-poisoned answer
    call(gateway(store, Inner(reported), policy=POLICY, profile=_profile()))
    meter = store.meter_get(T, J, S, A)
    assert meter is not None and meter["cost_usd"] == Decimal("0.00026")
    row = ledger(store)[0]
    assert row["price_mismatch"] is True and row["gateway_cost_usd"] == Decimal(1500)
