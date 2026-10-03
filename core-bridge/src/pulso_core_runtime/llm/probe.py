"""`llm_gateway` readiness: an authenticated, empty-body `POST /v1/generate`. The gateway checks method, auth,
content-type, size and only then decodes the body, so a valid token with an empty body is answered 400 (no provider
call, no cost) and a wrong token 401. `/healthz` proves nothing about auth. Cached so `/readyz` polling does not hit
the gateway each time; a failure is cached for a shorter time so recovery is noticed quickly. The token is only ever
sent in the Authorization header and never logged."""

from __future__ import annotations

import logging
import threading
import time
from collections.abc import Callable
from typing import Any

import httpx

_LOG = logging.getLogger("pulso_core_runtime.llm")


class GatewayProbe:
    def __init__(self, base_url: str, token: str, *, client: httpx.Client | None = None, timeout_s: float = 2.0,
                 ttl_s: float = 15.0, fail_ttl_s: float = 5.0, clock: Callable[[], float] = time.monotonic) -> None:
        self._url = base_url.rstrip("/") + "/v1/generate"
        self._headers = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}
        self._client = client if client is not None else httpx.Client()
        self._timeout, self._ttl, self._fail_ttl, self._clock = timeout_s, ttl_s, fail_ttl_s, clock
        self._lock = threading.Lock()
        self._at: float | None = None
        self._ok = False
        self._warned_auth = False
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
            response = self._client.post(self._url, content=b"", headers=self._headers,
                                         timeout=httpx.Timeout(self._timeout))
        except Exception:  # noqa: BLE001 - any transport failure is "not ready"; its text is never logged
            self.state = "unreachable"
            return False
        if response.status_code == 400:
            self.state, self._warned_auth = "ok", False
            return True
        if response.status_code == 401:
            self.state = "auth_failed"
            if not self._warned_auth:
                self._warned_auth = True
                _LOG.warning("llm_gateway_auth_failed")
            return False
        self.state = "unexpected_status"
        return False


def llm_gateway_check(base_url: str, token: str, **kw: Any) -> tuple[str, Callable[[], bool]]:
    return ("llm_gateway", GatewayProbe(base_url, token, **kw).check)
