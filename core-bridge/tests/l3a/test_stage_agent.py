"""Stage <-> agent pairing is enforced against `stages.catalog.CATALOG` before any receipt or Core call."""

from __future__ import annotations

import pytest

from pulso_core_runtime.stages.catalog import CATALOG

from pulso_core_runtime.internal.store import ensure_schema
from pulso_core_runtime.store.migrations import apply_l3

from .conftest import PgDbs
from .helpers import body, build_service, idem_key

pytestmark = [pytest.mark.l3a, pytest.mark.pg, pytest.mark.anyio]


@pytest.mark.parametrize("stage", sorted(CATALOG))
async def test_every_stage_refuses_a_foreign_agent_with_zero_effects(pg: PgDbs, stage: str) -> None:
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)
    svc, _core = build_service(pg.runtime)
    wrong = next(s.agent_id for s in CATALOG.values() if s.stage != stage)
    key = idem_key("t1", "j1", stage, 1, "k")
    out = await svc.invoke("t1", key, body(stage=stage, agent_id=wrong, input={}))
    assert out.status == 422 and out.body["code"] == "pulso:stage_agent_mismatch", out.body
    assert out.body["details"] == {"stage": stage, "agent_id": wrong}
    assert svc._store.get("t1", key) is None  # no receipt row
