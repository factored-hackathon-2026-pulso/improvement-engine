"""`build_evaluation_runtime` over a ServePorts-shaped stub (real stores) and DB isolation guard."""

from __future__ import annotations

from types import SimpleNamespace

import psycopg
import pytest
from agent_core.adapters.postgres_uow import PostgresStore
from agent_core.registry import PgRegistryStore
from testing.fakes.transcript import InMemoryTranscript

from l5.test_evaluate_path import FakeBroker, FixedBudgets
from l5.world import common_kwargs
from pulso_core_runtime.evaluation.isolated_registry import IsolationError, assert_isolated, isolated_registry_service
from pulso_core_runtime.registry_service import build_evaluation_runtime

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


def _ports(pg) -> SimpleNamespace:  # type: ignore[no-untyped-def]
    kw = common_kwargs()
    ev = PostgresStore(pg.eval)
    rt = PostgresStore(pg.runtime)
    api = SimpleNamespace(store=PgRegistryStore(lambda: psycopg.connect(pg.runtime, autocommit=False)),
                          eval_uow_factory=ev.uow, eval_audit=ev.audit())
    return SimpleNamespace(registry_api=api, clock=kw["clock"], ids=kw["ids"], keys=kw["keys"],
                           gateway=kw["gateway"], providers={}, calibrations=kw["calibrations"],
                           transcript=InMemoryTranscript(), classifier=kw["classifier"], uow_factory=rt.uow)


def test_build_runtime_shared_service_is_fail_closed_and_isolated(pg) -> None:  # type: ignore[no-untyped-def]
    rt = build_evaluation_runtime(_ports(pg), runtime_dsn=pg.runtime, eval_dsn=pg.eval, broker=FakeBroker(),
                                  budgets=FixedBudgets())
    assert rt.service is not None
    with pytest.raises(IsolationError):
        build_evaluation_runtime(_ports(pg), runtime_dsn=pg.runtime, eval_dsn=pg.runtime, broker=FakeBroker(),
                                 budgets=FixedBudgets())


def test_assert_isolated_uses_the_live_database_name(pg) -> None:  # type: ignore[no-untyped-def]
    assert_isolated(pg.runtime, pg.eval)
    with pytest.raises(IsolationError):
        assert_isolated(pg.runtime, pg.runtime + "?application_name=x")  # same DB, different URL spelling
    with pytest.raises(IsolationError, match="unreachable"):
        assert_isolated(pg.runtime, pg.admin.rpartition("/")[0] + "/no_such_db_l5")


def test_evolution_task_registry_is_isolated_and_cannot_evaluate() -> None:
    from agent_core.registry import EvalRequest  # noqa: F401
    from testing.fakes.clock import FakeClock
    from testing.fakes.ids import FakeIds

    from l5.world import bot_actor
    from agent_core.registry.models import Origin

    service, store = isolated_registry_service(FakeClock(), FakeIds())
    p = service.create_proposal(bot_actor(), "atencion", Origin.manual, "scratch")
    assert service.get_proposal(p.proposal_id).proposal.state.value == "draft"
    assert type(store).__name__ == "InMemoryRegistryStore"
