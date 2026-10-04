"""Python JevDecisionPort-style transport: the same contract as agent-core's `JevTransport`
(`send(request, timeout_ms) -> response`; `TimeoutError` when the budget is exhausted; `JevTransportError`
carrying only the HTTP status). Posts to the llm-gateway `/v1/jev` (or the roleplay server directly).
The API key is injected as a callable and only ever goes in the Authorization header. Redirects are never
followed and nothing from the request or response body is put in an exception."""
from __future__ import annotations

import json
import urllib.error
import urllib.request
from collections.abc import Callable
from typing import Any

_MAX_RESPONSE = 1 << 20


class JevTransportError(Exception):
    def __init__(self, status: int | None = None) -> None:
        super().__init__(f"jev: HTTP {status}" if status is not None else "jev: no HTTP response")
        self.status = status


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_: Any) -> None:
        return None


class HttpJevTransport:
    def __init__(self, base_url: str, api_key: Callable[[], str], *, path: str = "/v1/jev") -> None:
        self._url = base_url.rstrip("/") + path
        self._api_key = api_key
        self._opener = urllib.request.build_opener(_NoRedirect)

    def __repr__(self) -> str:
        return "HttpJevTransport()"

    def send(self, request: dict[str, Any], timeout_ms: int) -> dict[str, Any]:
        headers = {"Content-Type": "application/json", "Accept": "application/json"}
        key = self._api_key().strip()
        if key:
            headers["Authorization"] = f"Bearer {key}"
        http_request = urllib.request.Request(self._url, data=json.dumps(request).encode(), method="POST",
                                              headers=headers)
        try:
            with self._opener.open(http_request, timeout=timeout_ms / 1000) as response:
                payload = response.read(_MAX_RESPONSE + 1)
        except urllib.error.HTTPError as exc:
            status = exc.code
            exc.close()
            if status == 504:
                raise TimeoutError from None
            raise JevTransportError(status) from None
        except TimeoutError:
            raise TimeoutError from None
        except urllib.error.URLError as exc:
            if isinstance(exc.reason, TimeoutError):
                raise TimeoutError from None
            raise JevTransportError() from None
        except OSError:
            raise JevTransportError() from None
        try:
            decoded = json.loads(payload) if len(payload) <= _MAX_RESPONSE else None
        except ValueError:
            decoded = None
        if not isinstance(decoded, dict):
            raise JevTransportError() from None
        return decoded
