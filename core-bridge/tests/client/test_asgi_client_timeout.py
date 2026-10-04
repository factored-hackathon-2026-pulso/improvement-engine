"""CLT0: configurable invoke timeout of the in-process Core client (default 600 s, 1800 s for live DEMO-0 runs).
`httpx.ASGITransport` ignores httpx timeouts, so the client must enforce the deadline itself."""

from __future__ import annotations

import asyncio
from typing import Any

import httpx
import pytest

from pulso_core_runtime.client.asgi import AsgiCoreClient, CoreCallTimeout, timeout_from_env

pytestmark = pytest.mark.anyio


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


def _app(delay: float, effects: list[str] | None = None, stored: dict[str, Any] | None = None):
    async def app(scope: dict, receive: Any, send: Any) -> None:
        await receive()
        key = dict(scope["headers"]).get(b"idempotency-key", b"").decode()
        if stored is not None and key in stored:
            body = stored[key]
        else:
            if effects is not None:
                effects.append(key)  # the effect lands before the slow tail
            if stored is not None:
                stored[key] = b'{"run_id":"r1"}'
            await asyncio.sleep(delay)
            body = b'{"run_id":"r1"}'
        await send({"type": "http.response.start", "status": 201, "headers": [(b"content-type", b"application/json")]})
        await send({"type": "http.response.body", "body": body})
    return app


def test_default_timeout_is_600() -> None:
    assert AsgiCoreClient(lambda: None).timeout_s == 600.0


def test_timeout_from_env_and_validation() -> None:
    assert timeout_from_env({}) == 600.0
    assert timeout_from_env({"PULSO_CORE_INVOKE_TIMEOUT_S": "1800"}) == 1800.0
    for bad in ("0", "-5", "abc", "nan", "inf"):
        with pytest.raises(ValueError):
            timeout_from_env({"PULSO_CORE_INVOKE_TIMEOUT_S": bad})


async def test_call_over_timeout_is_cut_with_typed_error() -> None:
    client = AsgiCoreClient(lambda: _app(0.5), timeout_s=0.05)
    with pytest.raises(CoreCallTimeout) as exc:
        await client.start_run("b", "k", {})
    assert isinstance(exc.value, httpx.TimeoutException)  # the service maps any post-send exception to `unknown`


async def test_call_under_timeout_succeeds() -> None:
    client = AsgiCoreClient(lambda: _app(0.01), timeout_s=5)
    resp = await client.start_run("b", "k", {})
    assert resp.status == 201 and resp.body == {"run_id": "r1"}


@pytest.mark.parametrize("case", ["a", "b", "c"])
async def test_timeout_then_retry_same_key_has_no_duplicate_effect(case: str) -> None:
    effects: list[str] = []
    stored: dict[str, Any] = {}
    client = AsgiCoreClient(lambda: _app(0.3, effects, stored), timeout_s=0.05)
    with pytest.raises(CoreCallTimeout):
        await client.start_run("b", f"key-{case}", {})
    resp = await client.start_run("b", f"key-{case}", {})
    assert resp.status == 201
    assert effects == [f"key-{case}"]
