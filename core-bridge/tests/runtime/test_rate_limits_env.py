"""Pin 894fa65 (PR #29): operators tune Core's per-principal limits through AGENTCORE_RATE_* env; the runtime must
honour them instead of hard-coding `RateLimitConfig()`. Still works against 789d6c8 (no `rate_limits_from_env`)."""

from __future__ import annotations

from decimal import Decimal
from typing import Any

import pytest
from agent_core.api.limits import RateLimitConfig

from pulso_core_runtime import main as runtime_main
from pulso_core_runtime.internal.store import ensure_schema

from .conftest import PgDbs
from .test_pg_runtime import _compose, _env, keys  # noqa: F401

pytestmark = pytest.mark.runtime


def test_limits_from_env_reads_the_core_knobs() -> None:
    cfg = runtime_main.limits_from_env({"AGENTCORE_RATE_MAX_HITS": "7", "AGENTCORE_DAILY_BUDGET_USD": "1.50"})
    assert cfg.max_hits == 7 and cfg.daily_budget_usd == Decimal("1.50")
    assert runtime_main.limits_from_env({}) == RateLimitConfig()


def test_invalid_limit_value_is_a_config_error() -> None:
    with pytest.raises(ValueError):
        runtime_main.limits_from_env({"AGENTCORE_RATE_MAX_HITS": "many"})


def test_old_pin_without_rate_limits_from_env_keeps_the_defaults(monkeypatch: pytest.MonkeyPatch) -> None:
    import agent_core.composition.serve as serve
    monkeypatch.delattr(serve, "rate_limits_from_env")
    assert runtime_main.limits_from_env({"AGENTCORE_RATE_MAX_HITS": "7"}) == RateLimitConfig()


@pytest.mark.pg
def test_composed_app_applies_env_limits(pg: PgDbs, keys: dict[str, Any]) -> None:  # noqa: F811
    ensure_schema(pg.runtime)
    code, app, err = _compose(_env(pg, keys, AGENTCORE_RATE_MAX_HITS="7"))
    assert code == 0, err
    assert app.state.pulso_limits.max_hits == 7


@pytest.mark.pg
def test_composed_app_rejects_invalid_limit_env_before_serving(pg: PgDbs, keys: dict[str, Any]) -> None:  # noqa: F811
    ensure_schema(pg.runtime)
    code, app, err = _compose(_env(pg, keys, AGENTCORE_RATE_MAX_HITS="many"))
    assert code == runtime_main.EXIT_CONFIG and app is None and "AGENTCORE_RATE_MAX_HITS" in err
