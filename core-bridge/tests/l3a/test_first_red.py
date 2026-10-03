"""L3a first RED (plan 17.3.3): same key + other digest -> 409 with NO second start_run;
timeout after effect -> `unknown` (never `failed`), never re-executed."""

from __future__ import annotations

import pytest

from pulso_core_runtime.store.migrations import apply_l3

from .conftest import PgDbs
from .helpers import body, build_service, idem_key

pytestmark = [pytest.mark.l3a, pytest.mark.pg, pytest.mark.anyio]


@pytest.fixture
def dsn(pg: PgDbs) -> str:
    from pulso_core_runtime.internal.store import ensure_schema
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)
    return pg.runtime


async def test_same_key_other_digest_is_409_without_second_start_run(dsn: str) -> None:
    svc, core = build_service(dsn)
    key = idem_key("t1", "j1", "scout", 1, "k")
    first = await svc.invoke("t1", key, body())
    assert first.status == 200 and first.body["state"] == "terminal_ok"
    other = await svc.invoke("t1", key, body(input={"q": "DIFFERENT"}))
    assert other.status == 409 and other.body["code"] == "pulso:digest_conflict"
    assert len(core.start_calls) == 1


async def test_timeout_after_effect_is_unknown_and_never_reexecuted(dsn: str) -> None:
    svc, core = build_service(dsn)
    core.mode = "timeout_after_effect"
    key = idem_key("t1", "j1", "scout", 1, "k")
    out = await svc.invoke("t1", key, body())
    assert out.body["state"] == "unknown" and out.status == 202
    core.mode = "ok"
    again = await svc.invoke("t1", key, body())  # same key, same digest, not terminal -> re-read only
    assert again.body["state"] in {"unknown", "terminal_ok"}
    assert len(core.start_calls) == 1
