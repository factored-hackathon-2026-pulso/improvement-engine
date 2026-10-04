"""In-process Core client: the full M9 route (`POST /v1/runs`) through `httpx.ASGITransport`, never a direct
`TurnEngine` call (plan 17.3.3 step 7).

CLT0: the invoke timeout is configurable (default 600 s; `PULSO_CORE_INVOKE_TIMEOUT_S`, 1800 s for live DEMO-0
runs). `ASGITransport` ignores httpx timeouts, so the deadline is enforced here. A call that exceeds it raises
`CoreCallTimeout`; `InvokeService` maps any post-send exception to receipt state `unknown`, and re-entry with the
same key reconciles (re-reads) instead of re-executing."""

from __future__ import annotations

import asyncio
import math
from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any, Protocol

import httpx

DEFAULT_TIMEOUT_S = 600.0
TIMEOUT_ENV = "PULSO_CORE_INVOKE_TIMEOUT_S"


@dataclass(frozen=True)
class CoreResponse:
    status: int
    body: dict[str, Any]


class CoreRuns(Protocol):
    async def start_run(self, bearer: str, key: str, body: dict[str, Any]) -> CoreResponse: ...


class CoreCallTimeout(httpx.ReadTimeout):
    """The Core call outlived the configured invoke timeout; the effect may or may not have landed."""


def timeout_from_env(env: Mapping[str, str]) -> float:
    raw = env.get(TIMEOUT_ENV)
    if raw is None or raw.strip() == "":
        return DEFAULT_TIMEOUT_S
    try:
        value = float(raw)
    except ValueError:
        raise ValueError(f"{TIMEOUT_ENV} must be a positive number of seconds") from None
    if not math.isfinite(value) or value <= 0:
        raise ValueError(f"{TIMEOUT_ENV} must be a positive number of seconds")
    return value


class AsgiCoreClient:
    """`app_getter` is late-bound: the internal sub-app is mounted inside the very app it calls."""

    def __init__(self, app_getter: Any, timeout_s: float = DEFAULT_TIMEOUT_S) -> None:
        self._app_getter, self._timeout = app_getter, timeout_s

    @property
    def timeout_s(self) -> float:
        return self._timeout

    async def start_run(self, bearer: str, key: str, body: dict[str, Any]) -> CoreResponse:
        transport = httpx.ASGITransport(app=self._app_getter())
        try:
            async with asyncio.timeout(self._timeout):
                async with httpx.AsyncClient(transport=transport, base_url="http://core.local",
                                             timeout=self._timeout) as client:
                    resp = await client.post("/v1/runs", json=body, headers={
                        "Authorization": f"Bearer {bearer}", "Idempotency-Key": key})
        except TimeoutError:
            raise CoreCallTimeout(f"core invoke exceeded {self._timeout:g}s") from None
        try:
            data = resp.json()
        except ValueError:
            data = {}
        return CoreResponse(resp.status_code, data if isinstance(data, dict) else {})
