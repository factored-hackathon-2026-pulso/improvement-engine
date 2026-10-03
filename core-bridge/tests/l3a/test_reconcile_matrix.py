"""Unknown-case matrix (plan 17.3.3): every row is a recorded double over real PG receipts. Reconciliation
never starts a run and never writes to the registry (the probes are read-only by construction)."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

import pytest

from pulso_core_runtime.invoke.projection import FactSpec, WhitelistProjector
from pulso_core_runtime.reconcile.reconciler import Reconciler
from pulso_core_runtime.store.migrations import apply_l3
from pulso_core_runtime.store.receipts import ReceiptStore

from .conftest import PgDbs
from .fakes import FakeRunState

pytestmark = [pytest.mark.l3a, pytest.mark.pg]


@dataclass
class Runs:
    stored: dict[tuple[str, str], tuple[str, dict[str, Any]]] = field(default_factory=dict)
    runs: dict[str, Any] = field(default_factory=dict)

    def load_run(self, run_id: str) -> Any:
        return self.runs.get(run_id)

    def get_run_idempotency(self, principal_id: str, key: str) -> Any:
        return self.stored.get((principal_id, key))


@dataclass
class Writes:
    present: dict[str, dict[str, Any]] = field(default_factory=dict)
    fail: bool = False
    calls: list[str] = field(default_factory=list)

    def get_write(self, key: str) -> Any:
        self.calls.append(key)
        if self.fail:
            raise RuntimeError("5xx")
        return self.present.get(key)


@dataclass
class Bindings:
    rows: dict[tuple[str, str], dict[str, Any]] = field(default_factory=dict)
    fail: bool = False

    def lookup(self, tenant_id: str, command_key: str) -> Any:
        if self.fail:
            raise RuntimeError("timeout")
        return self.rows.get((tenant_id, command_key))


RESULT = {"run_id": "run-7", "release": "r", "outcome": "completed", "status": "done", "trace_id": "t"}


@pytest.fixture
def store(pg: PgDbs) -> ReceiptStore:
    from pulso_core_runtime.internal.store import ensure_schema
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)
    return ReceiptStore(pg.runtime)


def _sent(store: ReceiptStore, state: str = "sent", stage: str = "writer") -> Any:
    store.begin(tenant_id="t", key="k", digest="d", stage=stage, job_id="j", attempt=1, release_id="r",
                task_binding_ref="ref", principal_id="p")
    if state != "prepared":
        store.transition("t", "k", "sent")
    if state in ("binding_confirmed",):
        store.transition("t", "k", "binding_confirmed")
    return store.get("t", "k")


def _rec(store: ReceiptStore, runs: Runs, writes: Writes, bindings: Bindings, **kw: Any) -> Reconciler:
    return Reconciler(store=store, runs=runs, writes=writes, bindings=bindings,
                      expected_writes=lambda r: ["w-create", "w-put"], **kw)


def test_core_201_but_process_died_stored_result_wins(store: ReceiptStore) -> None:
    runs = Runs(stored={("p", "k"): ("h", RESULT)}, runs={"run-7": FakeRunState("run-7")})
    res = _rec(store, runs, Writes(), Bindings()).reconcile(_sent(store, "binding_confirmed"))
    assert (res.state, res.core_run_id) == ("terminal_ok", "run-7")


def test_completed_run_with_unconfirmed_binding_is_never_terminal_ok(store: ReceiptStore) -> None:
    runs = Runs(stored={("p", "k"): ("h", RESULT)}, runs={"run-7": FakeRunState("run-7")})
    res = _rec(store, runs, Writes(), Bindings()).reconcile(_sent(store, stage="scout"))
    assert (res.state, res.reason) == ("manual_reconcile", "binding_unconfirmed")


def test_writer_failed_outcome_after_effects_is_manual_reconcile_other_stages_terminal_failed(
        store: ReceiptStore) -> None:
    runs = Runs(stored={("p", "k"): ("h", {**RESULT, "outcome": "failed"})})
    assert _rec(store, runs, Writes(), Bindings()).reconcile(_sent(store)).state == "manual_reconcile"


def test_timeout_after_effect_with_committed_run_adopts_failed_outcome(store: ReceiptStore) -> None:
    runs = Runs(stored={("p", "k"): ("h", {**RESULT, "outcome": "failed"})})
    assert _rec(store, runs, Writes(), Bindings()).reconcile(_sent(store, stage="scout")).state == "terminal_failed"


def test_deadline_exceeded_stays_failed_not_unknown(store: ReceiptStore) -> None:
    runs = Runs(stored={("p", "k"): ("h", {**RESULT, "outcome": "failed", "status": "deadline_exceeded"})})
    assert _rec(store, runs, Writes(), Bindings()).reconcile(_sent(store, stage="scout")).state == "terminal_failed"


def test_release_drift_in_stored_result_is_terminal_failed(store: ReceiptStore) -> None:
    runs = Runs(stored={("p", "k"): ("h", {**RESULT, "release": "other"})})
    res = _rec(store, runs, Writes(), Bindings()).reconcile(_sent(store))
    assert res.state == "terminal_failed" and res.reason == "release_drift"


def test_binding_200_then_crash_no_writes_reconciled_budget_proves_no_effect(store: ReceiptStore) -> None:
    store.meter_add("t", "j", "writer", 1, calls=0)
    store.meter_reconcile("t", "j", "writer", 1)
    writes = Writes()
    res = _rec(store, Runs(), writes, Bindings({("t", "k"): {"core_run_id": "run-7"}})).reconcile(_sent(store))
    assert res.state == "terminal_failed" and res.proven_no_effect is True
    assert writes.calls == ["w-create", "w-put"]
    row = store.get("t", "k")
    assert row is not None and row.receipt == {"proven_no_effect": True}


def test_binding_then_crash_budget_unreconciled_is_manual(store: ReceiptStore) -> None:
    res = _rec(store, Runs(), Writes(), Bindings({("t", "k"): {"core_run_id": "run-7"}})).reconcile(_sent(store))
    assert (res.state, res.reason, res.proven_no_effect) == ("manual_reconcile", "budget_unreconciled", False)


def test_write_executed_with_lost_commit_is_adopted_and_verified(store: ReceiptStore) -> None:
    writes = Writes(present={"w-create": {"proposal_id": "p1"}})
    verified: list[Any] = []
    rec = _rec(store, Runs(), writes, Bindings({("t", "k"): {"core_run_id": "run-7"}}),
               commitment_check=lambda r, w: verified.append(w) or True)
    res = rec.reconcile(_sent(store))
    assert res.state == "manual_reconcile" and res.adopted_writes == ["w-create"] and verified
    assert res.proven_no_effect is False


def test_adopted_write_that_breaks_the_commitment_is_flagged(store: ReceiptStore) -> None:
    rec = _rec(store, Runs(), Writes(present={"w-put": {"x": 1}}), Bindings({("t", "k"): {"core_run_id": "r"}}),
               commitment_check=lambda r, w: False)
    assert rec.reconcile(_sent(store)).reason == "commitment_mismatch"


def test_get_write_5xx_stays_unknown(store: ReceiptStore) -> None:
    rec = _rec(store, Runs(), Writes(fail=True), Bindings({("t", "k"): {"core_run_id": "run-7"}}))
    res = rec.reconcile(_sent(store))
    assert res.state == "unknown" and res.reason == "get_write_unavailable"
    assert store.get("t", "k").state == "unknown"  # type: ignore[union-attr]


def test_binding_lookup_timeout_stays_unknown_and_sent_without_binding_is_manual(store: ReceiptStore) -> None:
    assert _rec(store, Runs(), Writes(), Bindings(fail=True)).reconcile(_sent(store)).state == "unknown"
    res = _rec(store, Runs(), Writes(), Bindings()).reconcile(store.get("t", "k"))  # type: ignore[arg-type]
    assert (res.state, res.reason) == ("manual_reconcile", "sent_without_binding")


def test_prepared_never_sent_is_proven_no_effect(store: ReceiptStore) -> None:
    res = _rec(store, Runs(), Writes(), Bindings()).reconcile(_sent(store, "prepared"))
    assert res.state == "terminal_failed" and res.proven_no_effect and res.reason == "never_sent"


def test_terminal_receipts_are_never_reopened(store: ReceiptStore) -> None:
    rec = _rec(store, Runs(stored={("p", "k"): ("h", RESULT)}), Writes(), Bindings())
    assert rec.reconcile(_sent(store, "binding_confirmed")).state == "terminal_ok"
    other = _rec(store, Runs(), Writes(), Bindings())
    assert other.reconcile(store.get("t", "k")).state == "terminal_ok"  # type: ignore[arg-type]


def test_adopted_result_with_projection_missing_fact_fails_the_stage(store: ReceiptStore) -> None:
    proj = WhitelistProjector({"writer": {"pulso_writer_receipts": FactSpec()}})
    runs = Runs(stored={("p", "k"): ("h", RESULT)}, runs={"run-7": FakeRunState("run-7")})
    res = _rec(store, runs, Writes(), Bindings(), projector=proj).reconcile(_sent(store, "binding_confirmed"))
    assert res.state == "terminal_failed" and res.reason == "output_missing"
