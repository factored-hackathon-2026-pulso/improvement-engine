"""Agent Core 894fa65 (PR #29): a second `start_run` with the same key while the first is still running (or crashed
and holds the reservation for `lease_ttl` = 60 s) answers `409 idempotency_conflict`. That is NOT "another body": the
bridge must wait and retry within the lease, never park a healthy job in `manual_reconcile`."""

from __future__ import annotations

import pytest

from pulso_core_runtime.invoke.service import InvokeSettings
from pulso_core_runtime.store.migrations import apply_l3

from .conftest import PgDbs
from .fakes import FakeCore
from .helpers import body, build_service, idem_key

pytestmark = [pytest.mark.l3a, pytest.mark.pg, pytest.mark.anyio]


@pytest.fixture
def dsn(pg: PgDbs) -> str:
    from pulso_core_runtime.internal.store import ensure_schema
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)
    return pg.runtime


def _service(dsn: str, core: FakeCore, sleeps: list[float]):  # type: ignore[no-untyped-def]
    async def fake_sleep(seconds: float) -> None:
        sleeps.append(seconds)

    return build_service(dsn, core, settings=InvokeSettings(max_inflight=8), sleep=fake_sleep)


async def test_in_flight_409_is_waited_out_then_the_stored_result_is_used(dsn: str) -> None:
    core, sleeps = FakeCore(), []
    core.in_flight_for = 2
    svc, _ = _service(dsn, core, sleeps)
    key = idem_key("t1", "j1", "scout", 1, "k")
    out = await svc.invoke("t1", key, body())
    assert out.status == 200 and out.body["state"] == "terminal_ok"
    assert len(core.start_calls) == 3 and {c["key"] for c in core.start_calls} == {key}
    assert len(sleeps) == 2 and sum(sleeps) < 60


async def test_in_flight_409_that_outlives_the_lease_is_unknown_not_manual_reconcile(dsn: str) -> None:
    core, sleeps = FakeCore(), []
    core.in_flight_for = 10_000
    svc, _ = _service(dsn, core, sleeps)
    key = idem_key("t1", "j1", "scout", 1, "k")
    out = await svc.invoke("t1", key, body())
    assert out.status == 202 and out.body["state"] == "unknown", out.body
    assert out.body["reason"] == "core_idempotency_in_flight"
    assert 60 <= sum(sleeps) <= 62, "waits for the whole lease (60 s) and no longer"
    assert len(core.start_calls) <= 40, "bounded retries (backoff), no hot loop"
    again = await svc.invoke("t1", key, body())  # a later identical request only re-reads, never re-sends
    assert again.body["state"] in {"unknown", "terminal_ok"}


async def test_core_409_without_the_in_flight_marker_still_means_another_body(dsn: str) -> None:
    core, sleeps = FakeCore(), []
    core.plain_conflict = True
    svc, _ = _service(dsn, core, sleeps)
    out = await svc.invoke("t1", idem_key("t1", "j1", "scout", 1, "k"), body())
    assert out.status == 409 and out.body["state"] == "manual_reconcile"
    assert out.body["reason"] == "core_idempotency_conflict" and out.body["code"] == "pulso:digest_conflict"
    assert sleeps == [] and len(core.start_calls) == 1
