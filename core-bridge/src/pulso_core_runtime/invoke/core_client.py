"""In-process Core client: the full M9 route (`POST /v1/runs`) through `httpx.ASGITransport`, never a direct
`TurnEngine` call (plan 17.3.3 step 7)."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Protocol

import httpx


@dataclass(frozen=True)
class CoreResponse:
    status: int
    body: dict[str, Any]


class CoreRuns(Protocol):
    async def start_run(self, bearer: str, key: str, body: dict[str, Any]) -> CoreResponse: ...


class AsgiCoreClient:
    """`app_getter` is late-bound: the internal sub-app is mounted inside the very app it calls."""

    def __init__(self, app_getter: Any, timeout_s: float = 600.0) -> None:
        self._app_getter, self._timeout = app_getter, timeout_s

    async def start_run(self, bearer: str, key: str, body: dict[str, Any]) -> CoreResponse:
        transport = httpx.ASGITransport(app=self._app_getter())
        async with httpx.AsyncClient(transport=transport, base_url="http://core.local",
                                     timeout=self._timeout) as client:
            resp = await client.post("/v1/runs", json=body,
                                     headers={"Authorization": f"Bearer {bearer}", "Idempotency-Key": key})
        try:
            data = resp.json()
        except ValueError:
            data = {}
        return CoreResponse(resp.status_code, data if isinstance(data, dict) else {})
