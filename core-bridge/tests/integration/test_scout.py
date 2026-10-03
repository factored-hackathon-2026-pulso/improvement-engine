"""Scout stage through `POST /internal/v1/core-tasks/invoke` with the REAL pinned Core on PG16: real service JWT,
real M9 identity (bridge-signed run principal), real pin, real `pulso/bind_context` (HTTP to the loopback
control-api), real `pulso/wiki_read` (HTTP to the loopback broker), binding guard + capped spend meter around the
scripted gateway. Doubles: loopback control-api/broker, scripted gateway."""

from __future__ import annotations

from typing import Any

import psycopg
import pytest

from integration.conftest import WORLD, Composed
from l3a.helpers import body, idem_key

pytestmark = [pytest.mark.integration, pytest.mark.pg]


def _scout(c: Composed, logical: str = "k", **over: Any) -> tuple[str, dict[str, Any]]:
    over.setdefault("input", {"briefing_ref": "wiki/briefing.md"})
    over.setdefault("memory_snapshot_ref", "mem-1")
    over.setdefault("release_id", c.release_ids["scout"])
    return idem_key("t1", "j1", "scout", 1, logical), body(
        agent_id="pulso-scout", agent_version="1.0.0", logical=logical, **over)


def _post(c: Composed, key: str, payload: dict[str, Any]) -> Any:
    return c.client.post("/internal/v1/core-tasks/invoke", json=payload,
                         headers={**c.headers("core_task_invoke"), "Idempotency-Key": key})


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_scout_real_core_binds_then_stops_at_unvalidated_slot_and_replays(composed: Composed) -> None:
    """CURRENT TRUTH: the pinned Core keeps run-input slots `claimed`, and `resolve_path` only reads `validated`
    slots, so `pulso-scout`'s `read_briefing` (args `slots.briefing_ref`) escalates before any tool call. Every
    other link is real: pin, M9 identity, binding callback, receipt CAS, replay."""
    key, payload = _scout(composed)
    r = _post(composed, key, payload)
    assert r.status_code in (200, 202), r.text
    out = r.json()
    assert out["core_run_id"], out
    assert len(composed.loop.bindings) == 1  # real bind_context reached the control-api with Core's run id
    assert composed.loop.bindings[0]["core_run_id"] == out["core_run_id"]
    assert (out["state"], out["outcome"]) == ("terminal_failed", "escalated"), out
    assert composed.gateway.agent_calls == 0 and composed.loop.backend.wiki_requests == 0
    again = _post(composed, key, payload)  # same key + body: receipt replay, no second Core run or binding
    assert again.json()["core_run_id"] == out["core_run_id"] and len(composed.loop.bindings) == 1
    with psycopg.connect(composed.pg.runtime) as conn:
        assert conn.execute("SELECT count(*) FROM runs").fetchone()[0] == 1  # type: ignore[index]


@pytest.mark.xfail(strict=True, reason="pinned Core 1.3.0: run-input slots stay `claimed`; Flows read `slots.*` "
                   "(needs bind_context-provided facts or validated task inputs; reported to L4/L3b)")
@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_scout_happy_path_reaches_model_and_terminal_ok(composed: Composed) -> None:
    key, payload = _scout(composed, logical="happy")
    out = _post(composed, key, payload).json()
    assert composed.loop.backend.wiki_requests >= 1 and composed.gateway.agent_calls == 1
    assert out["state"] == "terminal_ok", out
    from pulso_core_runtime.store.receipts import ReceiptStore
    meter = ReceiptStore(composed.pg.runtime).meter_get("t1", "j1", "scout", 1)
    assert meter is not None and meter["calls"] == 1  # capped atomic spend meter charged the live call


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_binding_denied_never_reaches_the_model(composed: Composed) -> None:
    composed.loop.backend.bind_mode = "conflict"
    key, payload = _scout(composed, logical="den")
    out = _post(composed, key, payload).json()
    assert out["state"] != "terminal_ok" and composed.gateway.agent_calls == 0


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_wrong_purpose_and_tenant_are_refused(composed: Composed) -> None:
    key, payload = _scout(composed, logical="auth")
    bad = composed.client.post("/internal/v1/core-tasks/invoke", json=payload,
                               headers={**composed.headers("core_task_read"), "Idempotency-Key": key})
    assert bad.status_code in (401, 403)
    other = composed.client.post("/internal/v1/core-tasks/invoke", json=payload, headers={
        **composed.headers("core_task_invoke", tenant="t2"), "Idempotency-Key": key})
    assert other.status_code == 403 and other.json()["code"] == "pulso:tenant_mismatch"
    assert composed.loop.bindings == []
