"""Receipt store: migrations, single-flight insert, CAS state machine, terminal states final."""

from __future__ import annotations

import threading

import pytest

from pulso_core_runtime.store.migrations import apply_l3, l3_ready
from pulso_core_runtime.store.receipts import ALLOWED_FROM, TERMINAL, ReceiptStore

from .conftest import PgDbs

pytestmark = [pytest.mark.l3a, pytest.mark.pg]


@pytest.fixture
def store(pg: PgDbs) -> ReceiptStore:
    from pulso_core_runtime.internal.store import ensure_schema
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)
    apply_l3(pg.runtime)  # idempotent
    assert l3_ready(pg.runtime)
    return ReceiptStore(pg.runtime)


def _begin(store: ReceiptStore, key: str = "k", digest: str = "d"):
    return store.begin(tenant_id="t", key=key, digest=digest, stage="scout", job_id="j", attempt=1,
                       release_id="r", task_binding_ref="ref", principal_id="p")


def test_begin_is_single_flight_across_threads(store: ReceiptStore) -> None:
    results: list[bool] = []
    barrier = threading.Barrier(12)

    def go() -> None:
        barrier.wait()
        results.append(_begin(store)[1])

    threads = [threading.Thread(target=go) for _ in range(12)]
    [t.start() for t in threads]
    [t.join() for t in threads]
    assert results.count(True) == 1 and results.count(False) == 11


def test_cas_has_one_winner_and_terminal_is_final(store: ReceiptStore) -> None:
    _begin(store)
    assert store.transition("t", "k", "sent") is not None
    assert store.transition("t", "k", "sent") is None  # prepared -> sent only once
    wins: list[str] = []
    barrier = threading.Barrier(2)

    def race(target: str) -> None:
        barrier.wait()
        if store.transition("t", "k", target) is not None:
            wins.append(target)

    a = threading.Thread(target=race, args=("terminal_ok",))
    b = threading.Thread(target=race, args=("terminal_failed",))
    a.start()
    b.start()
    a.join()
    b.join()
    assert len(wins) == 1
    for target in ALLOWED_FROM:
        assert store.transition("t", "k", target) is None  # terminal: no outgoing edge
    row = store.get("t", "k")
    assert row is not None and row.state in TERMINAL and row.version == 2


def test_context_rows_and_budget_meter(store: ReceiptStore) -> None:
    from datetime import UTC, datetime, timedelta
    store.save_context("ref", "t", "j", "k", {"a": 1}, datetime.now(UTC) + timedelta(minutes=5))
    row = store.context_row("ref")
    assert row is not None and row["deleted_at"] is None
    store.delete_context("ref")
    row = store.context_row("ref")
    assert row is not None and row["deleted_at"] is not None
    store.meter_add("t", "j", "scout", 1, calls=1, tokens=10, cost_usd="0.5")
    store.meter_add("t", "j", "scout", 1, calls=2, tokens=5, cost_usd="0.25", usage_known=False)
    m = store.meter_get("t", "j", "scout", 1)
    assert m is not None and m["calls"] == 3 and m["tokens"] == 15 and str(m["cost_usd"]).startswith("0.75")
    assert m["usage_known"] is False and m["reconciled"] is False
    assert store.meter_reconcile("t", "j", "scout", 1) and not store.meter_reconcile("t", "j", "x", 1)
