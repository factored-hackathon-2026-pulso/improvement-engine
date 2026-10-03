"""`llm_gateway` readiness: a plain reachability signal (`GET /healthz` answered below 5xx). The token is not sent:
deep gateway-specific probing (auth, contract) is the gateway owner's concern. Cached so `/readyz` polling does not
hit the gateway each time; a failure is cached for a shorter time so recovery is noticed quickly."""

from __future__ import annotations

import threading
import time
from collections.abc import Callable
from typing import Any

import httpx


class GatewayProbe:
    def __init__(self, base_url: str, token: str | None = None, *, client: httpx.Client | None = None, timeout_s: float = 2.0,
                 ttl_s: float = 15.0, fail_ttl_s: float = 5.0, clock: Callable[[], float] = time.monotonic) -> None:
        self._url = base_url.rstrip("/") + "/healthz"
        self._client = client if client is not None else httpx.Client()
        self._timeout, self._ttl, self._fail_ttl, self._clock = timeout_s, ttl_s, fail_ttl_s, clock
        self._lock = threading.Lock()
        self._at: float | None = None
        self._ok = False
        self.state = "unknown"

    def check(self) -> bool:
        with self._lock:
            now = self._clock()
            if self._at is not None and now - self._at < (self._ttl if self._ok else self._fail_ttl):
                return self._ok
            self._ok = self._probe()
            self._at = now
            return self._ok

    def _probe(self) -> bool:
        try:
            response = self._client.get(self._url, timeout=httpx.Timeout(self._timeout))
        except Exception:  # noqa: BLE001 - any transport failure is "not ready"; its text is never logged
            self.state = "unreachable"
            return False
        if response.status_code < 500:
            self.state = "ok"
            return True
        self.state = "unexpected_status"
        return False


def llm_gateway_check(base_url: str, token: str, **kw: Any) -> tuple[str, Callable[[], bool]]:
    return ("llm_gateway", GatewayProbe(base_url, token, **kw).check)
