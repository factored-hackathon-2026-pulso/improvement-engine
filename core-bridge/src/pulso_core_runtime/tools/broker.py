"""HTTP clients for the Codex broker (annex D.3, prefix `/internal/v1/broker`) and the control-api binding
callback (CAP-27). Synchronous (tools run on engine threads). The caller never picks a tenant: identity travels in
the service JWT (`aud=lab-broker|control-api`); `binding_ref` is correlation only (an `X-Pulso-Binding-Ref`
header on GETs, a body field where D.3 lists it)."""

from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

import httpx

PREFIX = "/internal/v1/broker"
TokenProvider = Callable[[str], str]  # scope -> bearer JWS


class BrokerError(Exception):
    def __init__(self, status: int, code: str) -> None:
        super().__init__(f"broker {status} {code}")
        self.status, self.code = status, code


class BrokerTimeout(Exception):
    """Client timeout; the effect (if any) is unknown."""


class BrokerUnavailable(Exception):
    """Network failure before a response."""


@dataclass(frozen=True)
class AuthDecision:
    allowed: bool
    authorization_revision: int | None
    valid_until: datetime | None
    reason_code: str | None


def _parse_ts(value: Any) -> datetime | None:
    if not isinstance(value, str):
        return None
    try:
        return datetime.fromisoformat(value).astimezone(UTC)
    except ValueError:
        return None


class _Base:
    def __init__(self, base_url: str, tokens: TokenProvider, *, http: httpx.Client | None = None,
                 timeout_s: float = 30.0) -> None:
        self._http = http or httpx.Client(base_url=base_url, timeout=timeout_s)
        self._base = base_url.rstrip("/")
        self._tokens = tokens
        self._timeout = timeout_s

    def _send(self, method: str, path: str, scope: str, *, json: Any = None, params: dict[str, str] | None = None,
              binding_ref: str | None = None, idempotency_key: str | None = None,
              timeout_s: float | None = None) -> httpx.Response:
        headers = {"Authorization": "Bearer " + self._tokens(scope)}
        if binding_ref:
            headers["X-Pulso-Binding-Ref"] = binding_ref
        if idempotency_key:
            headers["Idempotency-Key"] = idempotency_key
        try:
            return self._http.request(method, self._base + path, json=json, params=params, headers=headers,
                                      timeout=timeout_s or self._timeout)
        except httpx.TimeoutException as exc:
            raise BrokerTimeout(path) from exc
        except httpx.TransportError as exc:
            raise BrokerUnavailable(path) from exc

    @staticmethod
    def _json(resp: httpx.Response) -> Any:
        if resp.status_code >= 400:
            code = "error"
            try:
                body = resp.json()
                code = str(body.get("code", code)) if isinstance(body, dict) else code
            except ValueError:
                pass
            raise BrokerError(resp.status_code, code)
        try:
            return resp.json()
        except ValueError as exc:
            raise BrokerError(resp.status_code, "invalid_json") from exc


class ControlApiClient(_Base):
    """`POST /internal/v1/core-task-bindings` (scope `binding`, 5 s timeout, one retry on network error)."""

    def __init__(self, base_url: str, tokens: TokenProvider, *, http: httpx.Client | None = None) -> None:
        super().__init__(base_url, tokens, http=http, timeout_s=5.0)

    def bind(self, body: dict[str, Any], idempotency_key: str) -> dict[str, Any]:
        last: Exception | None = None
        for _ in range(2):
            try:
                resp = self._send("POST", "/internal/v1/core-task-bindings", "binding", json=body,
                                  idempotency_key=idempotency_key, timeout_s=5.0)
                result = self._json(resp)
                return result if isinstance(result, dict) else {}
            except BrokerUnavailable as exc:  # network error -> one retry; timeouts are NOT retried
                last = exc
        assert last is not None
        raise last


class BrokerClient(_Base):
    def __init__(self, base_url: str, tokens: TokenProvider, *, http: httpx.Client | None = None,
                 timeout_s: float = 30.0, poll_interval_s: float = 0.5, poll_budget_s: float = 120.0,
                 sleep: Callable[[float], None] = time.sleep,
                 monotonic: Callable[[], float] = time.monotonic) -> None:
        super().__init__(base_url, tokens, http=http, timeout_s=timeout_s)
        self.poll_interval_s, self.poll_budget_s = poll_interval_s, poll_budget_s
        self._sleep, self._monotonic = sleep, monotonic

    def _call(self, method: str, suffix: str, binding_ref: str, *, json: Any = None,
              params: dict[str, str] | None = None, idempotency_key: str | None = None) -> Any:
        return self._json(self._send(method, PREFIX + suffix, "broker", json=json, params=params,
                                     binding_ref=binding_ref, idempotency_key=idempotency_key))

    # -- authorisation -------------------------------------------------------
    def authorization_check(self, binding_ref: str, operation: str, resource_refs: list[str],
                            payload_digest: str | None) -> AuthDecision:
        body = self._call("POST", "/authorizations/check", binding_ref, json={
            "binding_ref": binding_ref, "operation": operation, "resource_refs": resource_refs,
            "payload_digest": payload_digest})
        return AuthDecision(allowed=body.get("allowed") is True,
                            authorization_revision=body.get("authorization_revision"),
                            valid_until=_parse_ts(body.get("valid_until")), reason_code=body.get("reason_code"))

    # -- artifacts -----------------------------------------------------------
    def artifact_get(self, binding_ref: str, artifact_id: str, expected_digest: str | None = None) -> dict[str, Any]:
        params = {"digest": expected_digest} if expected_digest else None
        out = self._call("GET", f"/artifacts/{artifact_id}", binding_ref, params=params)
        return out if isinstance(out, dict) else {}

    # -- lab -------------------------------------------------------------------
    def lab_open_session(self, binding_ref: str, extract_manifest_ref: str) -> dict[str, Any]:
        return dict(self._call("POST", "/lab/sessions", binding_ref, json={
            "binding_ref": binding_ref, "extract_manifest_ref": extract_manifest_ref}))

    def lab_submit_query(self, binding_ref: str, session_ref: str, query_key: str, sql: str,
                         expected_session_revision: int) -> dict[str, Any]:
        return dict(self._call("POST", f"/lab/sessions/{session_ref}/queries", binding_ref, json={
            "query_key": query_key, "sql": sql, "expected_session_revision": expected_session_revision},
            idempotency_key=query_key))

    def lab_query_state(self, binding_ref: str, query_ref: str) -> dict[str, Any]:
        return dict(self._call("GET", f"/lab/queries/{query_ref}", binding_ref))

    def lab_wait(self, binding_ref: str, query_ref: str) -> dict[str, Any]:
        """Internal poll of `GET /lab/queries/{id}` bounded by `poll_budget_s` (<= 120 s)."""
        deadline = self._monotonic() + self.poll_budget_s
        while True:
            state = self.lab_query_state(binding_ref, query_ref)
            if state.get("state") in ("completed", "failed", "unknown"):
                return state
            if self._monotonic() >= deadline:
                raise BrokerTimeout("lab_query_poll_budget")
            self._sleep(self.poll_interval_s)

    def lab_result(self, binding_ref: str, result_ref: str, cursor: str | None = None) -> dict[str, Any]:
        return dict(self._call("GET", f"/lab/results/{result_ref}", binding_ref,
                               params={"cursor": cursor} if cursor else None))

    def lab_close(self, binding_ref: str, session_ref: str, reason: str, key: str) -> dict[str, Any]:
        return dict(self._call("POST", f"/lab/sessions/{session_ref}/close", binding_ref,
                               json={"reason": reason}, idempotency_key=key))

    # -- wiki ------------------------------------------------------------------
    def wiki_read(self, binding_ref: str, memory_snapshot_ref: str, paths: list[str]) -> dict[str, Any]:
        return dict(self._call("POST", "/wiki/read", binding_ref, json={
            "memory_snapshot_ref": memory_snapshot_ref, "paths": paths}))

    def wiki_explore(self, binding_ref: str, memory_snapshot_ref: str, path: str, query: str | None,
                     cursor: str | None = None) -> dict[str, Any]:
        return dict(self._call("POST", "/wiki/explore", binding_ref, json={
            "memory_snapshot_ref": memory_snapshot_ref, "path": path, "query": query, "cursor": cursor}))

    def wiki_transform(self, binding_ref: str, memory_snapshot_ref: str, base_digest: str,
                       operations: list[Any]) -> dict[str, Any]:
        return dict(self._call("POST", "/wiki/transform", binding_ref, json={
            "memory_snapshot_ref": memory_snapshot_ref, "base_digest": base_digest, "operations": operations}))
