"""Scout stage through `POST /internal/v1/core-tasks/invoke` with the REAL pinned Core on PG16: real service JWT,
real M9 identity (bridge-signed run principal), real pin, real `pulso/bind_context` (HTTP to the loopback
control-api), real `pulso/wiki_read` (HTTP to the loopback broker), binding guard + capped spend meter around the
scripted gateway. Doubles: loopback control-api/broker, scripted gateway."""

from __future__ import annotations

from decimal import Decimal
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


def _script_scout_model(c: Composed) -> None:
    """Double: the agent step first queries the Lab (real `pulso/lab_query` -> loopback broker), then answers `final`
    citing the result artifact the invocation really fetched (strict D.2 evidence)."""
    from agent_core.ports import GenerationResult
    from integration.conftest import SCOUT_OUTPUT
    original = c.gateway.generate

    def generate(prompt: Any, view: Any, locale: Any, schema: Any = None) -> Any:
        if not (isinstance(view, dict) and "goal" in view):
            return original(prompt, view, locale, schema)
        c.gateway.agent_calls += 1
        if c.gateway.agent_calls == 1:
            out: dict[str, Any] = {"kind": "tool_call", "tool": "pulso/lab_query@1.0.0", "args": {"sql": "select 1"}}
        else:
            hyp = {**SCOUT_OUTPUT["hypotheses"][0],
                   "evidence_refs": [{"id": "res-1", "digest": "e" * 64, "media_type": "application/json"}]}
            out = {"kind": "final", "output": {"schema_version": "1", "hypotheses": [hyp]}}
        return GenerationResult(output=out, tokens_in=10, tokens_out=5, cost_usd=Decimal("0.001"),
                                model="double-model")
    c.gateway.generate = generate  # type: ignore[method-assign]


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_scout_happy_path_reads_inputs_from_bind_facts_reaches_model_and_replays(composed: Composed) -> None:
    """Core 1.3.0 keeps run-input slots `claimed` (Flows cannot read `slots.*`), so `pulso/bind_context` re-exposes
    the declared inputs as tool-origin facts and `read_briefing` reads `facts.binding.value.briefing_ref`."""
    _script_scout_model(composed)
    key, payload = _scout(composed, logical="happy", extract_manifest_ref="ex-1")
    out = _post(composed, key, payload).json()
    assert len(composed.loop.bindings) == 1 and composed.loop.bindings[0]["core_run_id"] == out["core_run_id"]
    wiki = [b for r, b in zip(composed.loop.backend.requests, composed.loop.backend.bodies, strict=True)
            if r.url.path.endswith("/wiki/read") and b]
    assert wiki and wiki[0]["paths"] == ["wiki/briefing.md"]  # the bound input reached the tool as its argument
    assert composed.gateway.agent_calls == 2, out
    assert out["state"] == "terminal_ok", out
    from pulso_core_runtime.store.receipts import ReceiptStore
    meter = ReceiptStore(composed.pg.runtime).meter_get("t1", "j1", "scout", 1)
    assert meter is not None and meter["calls"] == 2  # capped atomic spend meter charged each live call
    again = _post(composed, key, payload)  # same key + body: receipt replay, no second Core run or binding
    assert again.json()["core_run_id"] == out["core_run_id"] and len(composed.loop.bindings) == 1
    with psycopg.connect(composed.pg.runtime) as conn:
        assert conn.execute("SELECT count(*) FROM runs").fetchone()[0] == 1  # type: ignore[index]


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
