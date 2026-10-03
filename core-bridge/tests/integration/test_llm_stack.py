"""`HttpLLMGateway` (agent-core) through `BindingGuardGateway(SpendMeteringGateway(...))` with the REAL pinned Core on
PG16. The gateway is a double following the documented llm-gateway contract (`tests/llm/gateway_double.py`; no real
gateway binary/image exists here)."""

from __future__ import annotations

from typing import Any

import pytest
from llm.gateway_double import GatewayDouble, ok

from integration.conftest import SCOUT_OUTPUT, WORLD, Composed
from integration.test_scout import _post, _scout
from pulso_core_runtime.store.receipts import ReceiptStore

pytestmark = [pytest.mark.integration, pytest.mark.pg]
URL, TOKEN = "http://llm-gateway.test:8080", "tok-ok"


def _wire_http_gateway(c: Composed, double: GatewayDouble) -> None:
    from agent_core.adapters.llm.http_gateway import HttpLLMGateway
    http = HttpLLMGateway(c.ports.registry, URL, TOKEN, client=double.client())
    original = c.gateway.generate

    def generate(prompt: Any, view: Any, locale: Any, schema: Any = None) -> Any:
        if not (isinstance(view, dict) and "goal" in view):
            return original(prompt, view, locale, schema)
        c.gateway.agent_calls += 1
        return http.generate(prompt, view, locale, schema)
    c.gateway.generate = generate  # type: ignore[method-assign]


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_http_gateway_success_is_metered_and_ledgered(composed: Composed) -> None:
    hyp = {**SCOUT_OUTPUT["hypotheses"][0], "evidence_refs": [{"id": "res-1", "digest": "e" * 64,
                                                               "media_type": "application/json"}]}
    double = GatewayDouble(
        ok(output={"kind": "tool_call", "tool": "pulso/lab_query@1.0.0", "args": {"sql": "select 1"}}),
        ok(output={"kind": "final", "output": {"schema_version": "1", "hypotheses": [hyp]}}))
    _wire_http_gateway(composed, double)
    key, payload = _scout(composed, logical="http-ok", extract_manifest_ref="ex-1")
    out = _post(composed, key, payload).json()
    assert out["state"] == "terminal_ok", out
    assert len(double.requests) == 2  # one POST per call, no hidden retries
    req = double.requests[0]
    assert req["profile"]["endpoint_alias"] and set(req["labels"]) <= {"prompt", "model_profile", "run_id", "turn_id",
                                                                       "session_id", "release", "agent"}
    store = ReceiptStore(composed.pg.runtime)
    meter = store.meter_get("t1", "j1", "scout", 1)
    assert meter is not None and meter["calls"] == 2 and meter["reserved_usd"] == 0
    rows = store.ledger_rows("t1", "j1", "scout", 1)
    assert [r["outcome"] for r in rows] == ["ok", "ok"] and all(r["model_reported"] == "reported-model" for r in rows)
