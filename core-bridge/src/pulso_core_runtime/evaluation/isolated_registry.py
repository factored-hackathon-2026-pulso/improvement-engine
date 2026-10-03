"""Isolation helpers (plan 17.3.5): eval DB separate from the operative registry; `evolution_task` registry.

* `assert_isolated(runtime_dsn, eval_dsn)`: the two DSNs must name different databases (checked with
  `current_database()` on a live connection, not by parsing the URL).
* `isolated_registry_service(...)`: an in-memory `RegistryService` for the `evolution_task` profile whose
  evaluator raises (no nested evaluation); nothing it does can reach the operational registry."""

from __future__ import annotations

from typing import Any

import psycopg
from agent_core.registry import EvalReport, EvalRequest, HarnessUnavailable, RegistryService
from agent_core.registry.memory import InMemoryRegistryStore


class IsolationError(RuntimeError):
    pass


def current_database(dsn: str) -> str:
    with psycopg.connect(dsn, autocommit=True, connect_timeout=3) as conn:
        row = conn.execute("SELECT current_database()").fetchone()
    assert row is not None
    return str(row[0])


def assert_isolated(runtime_dsn: str, eval_dsn: str) -> None:
    try:
        same = current_database(runtime_dsn) == current_database(eval_dsn)
    except psycopg.Error:
        raise IsolationError("pulso:eval_db_unreachable") from None
    if same:
        raise IsolationError("pulso:eval_db_not_isolated")


class _NoNestedEvaluation:
    def run(self, request: EvalRequest) -> EvalReport:
        raise HarnessUnavailable("nested_evaluation_forbidden")


def isolated_registry_service(clock: Any, ids: Any) -> tuple[RegistryService, Any]:
    store = InMemoryRegistryStore()
    return RegistryService(store, _NoNestedEvaluation(), clock, ids), store
