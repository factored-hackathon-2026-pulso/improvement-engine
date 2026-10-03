"""Broker + control-api test double GENERATED from annex D.3 (plan) and CAP-27 (V3).

`D3_ROUTES` is the annex table verbatim (method, suffix, request keys, response keys); the fake is a
route-table-driven `httpx.MockTransport`, so drift between the table and the client fails the tests.
Everything is recorded in `requests` so tests can assert "zero lab queries"."""

from __future__ import annotations

import json
import re
from datetime import UTC, datetime, timedelta
from typing import Any

import httpx

BROKER = "/internal/v1/broker"
# (method, suffix, request keys, response keys) -- annex D.3
D3_ROUTES: tuple[tuple[str, str, tuple[str, ...], tuple[str, ...]], ...] = (
    ("GET", "/artifacts/{id}", (), ("schema_version", "artifact", "encoding", "content", "byte_length")),
    ("POST", "/authorizations/check", ("binding_ref", "operation", "resource_refs", "payload_digest"),
     ("allowed", "authorization_revision", "valid_until", "reason_code")),
    ("POST", "/lab/sessions", ("binding_ref", "extract_manifest_ref"),
     ("session_ref", "revision", "manifest_digest", "limits", "table_catalog")),
    ("POST", "/lab/sessions/{id}/queries", ("query_key", "sql", "expected_session_revision"),
     ("query_ref", "status_url")),
    ("GET", "/lab/queries/{id}", (), ("state", "receipt_ref", "result_ref", "reason_code")),
    ("GET", "/lab/results/{id}", (), ("columns", "rows", "truncated", "next_cursor", "total_rows",
                                       "receipt_ref", "result_digest")),
    ("GET", "/lab/sessions/{id}", (), ("state", "revision", "current_query_ref", "limits")),
    ("POST", "/lab/sessions/{id}/close", ("reason",), ("state",)),
    ("POST", "/wiki/read", ("memory_snapshot_ref", "paths"), ("entries", "base_digest")),
    ("POST", "/wiki/explore", ("memory_snapshot_ref", "path", "query", "cursor"),
     ("entries", "next_cursor", "truncated")),
    ("POST", "/wiki/transform", ("memory_snapshot_ref", "base_digest", "operations"),
     ("scratch_ref", "diff_ref", "manifest_digest", "violations")),
)


def _regex(suffix: str) -> re.Pattern[str]:
    return re.compile("^" + suffix.replace("{id}", "([^/]+)") + "$")


class FakeBackend:
    """Mutable behaviour knobs + request log."""

    def __init__(self) -> None:
        self.requests: list[httpx.Request] = []
        self.bodies: list[Any] = []
        self.bind_mode = "ok"  # ok | conflict | digest_mismatch | not_found | unavailable | timeout | network_once
        self.auth_allowed = True
        self.auth_valid_for_s = 60
        self.auth_mode = "ok"  # ok | 5xx | timeout
        self.query_polls_before_done = 0
        self._polls = 0
        self.columns: list[dict[str, str]] = [
            {"name": "event_count", "type": "int", "data_class": "public"},
            {"name": "amount", "type": "decimal", "data_class": "financial"},
            {"name": "free_text", "type": "text", "data_class": "untrusted_text"}]
        self.rows: list[list[Any]] = [[1, "10.50", "hello"], [2, "3.00", "world"]]
        self.total_rows: int | None = 2
        self.truncated = False
        self.artifacts: dict[str, dict[str, Any]] = {}
        self.bound: list[tuple[str, str]] = []

    def count(self, fragment: str) -> int:
        return sum(1 for r in self.requests if fragment in r.url.path)

    @property
    def lab_requests(self) -> int:
        return self.count("/lab/")

    @property
    def wiki_requests(self) -> int:
        return self.count("/wiki/")

    @property
    def broker_effect_calls(self) -> int:
        return sum(1 for r in self.requests if r.url.path.startswith(BROKER) and "authorizations" not in r.url.path)

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        body = json.loads(request.content) if request.content else None
        self.bodies.append(body)
        path = request.url.path
        if path == "/internal/v1/core-task-bindings":
            return self._bind(request, body)
        if not path.startswith(BROKER):
            return httpx.Response(404, json={"code": "not_found"})
        suffix = path[len(BROKER):]
        for method, tmpl, req_keys, _resp in D3_ROUTES:
            m = _regex(tmpl).match(suffix)
            if m and method == request.method:
                if body is not None and not set(body) <= set(req_keys):
                    return httpx.Response(400, json={"code": "unknown_field"})
                name = "r_" + re.sub(r"\W+", "_", tmpl).strip("_")
                return getattr(self, name)(m, body)  # type: ignore[no-any-return]
        return httpx.Response(404, json={"code": "not_found"})

    def _bind(self, request: httpx.Request, body: dict[str, Any]) -> httpx.Response:
        if self.bind_mode == "timeout":
            raise httpx.ReadTimeout("bind timeout", request=request)
        if self.bind_mode == "network_once":
            self.bind_mode = "ok"
            raise httpx.ConnectError("boom", request=request)
        if self.bind_mode in ("conflict", "digest_mismatch"):
            return httpx.Response(409, json={"code": "binding_conflict" if self.bind_mode == "conflict"
                                             else "digest_mismatch"})
        if self.bind_mode == "not_found":
            return httpx.Response(404, json={"code": "not_found"})
        if self.bind_mode == "unavailable":
            return httpx.Response(503, json={"code": "unavailable"})
        self.bound.append((body["tenant_id"], body["job_id"]))
        return httpx.Response(200, json={"schema_version": "1", "state": "confirmed",
                                         "tenant_id": body["tenant_id"], "job_id": body["job_id"]})

    # -- D.3 routes (names derived from the suffix) ------------------------
    def r_authorizations_check(self, m: Any, body: dict[str, Any]) -> httpx.Response:
        if self.auth_mode == "timeout":
            raise httpx.ReadTimeout("auth timeout")
        if self.auth_mode == "5xx":
            return httpx.Response(503, json={"code": "unavailable"})
        until = (datetime.now(UTC) + timedelta(seconds=self.auth_valid_for_s)).strftime("%Y-%m-%dT%H:%M:%SZ")
        return httpx.Response(200, json={"allowed": self.auth_allowed, "authorization_revision": 1,
                                         "valid_until": until,
                                         "reason_code": None if self.auth_allowed else "revoked"})

    def r_artifacts_id(self, m: Any, body: Any) -> httpx.Response:
        art = self.artifacts.get(m.group(1))
        if art is None:
            return httpx.Response(404, json={"code": "artifact_not_found"})
        return httpx.Response(200, json=art)

    def r_lab_sessions(self, m: Any, body: dict[str, Any]) -> httpx.Response:
        return httpx.Response(200, json={"session_ref": "sess-1", "revision": 3, "manifest_digest": "d" * 64,
                                         "limits": {"max_rows": 200}, "table_catalog": []})

    def r_lab_sessions_id_queries(self, m: Any, body: dict[str, Any]) -> httpx.Response:
        ref = "q-" + body["query_key"][:8]
        return httpx.Response(202, json={"query_ref": ref, "status_url": "/lab/queries/" + ref})

    def r_lab_queries_id(self, m: Any, body: Any) -> httpx.Response:
        self._polls += 1
        if self._polls <= self.query_polls_before_done:
            return httpx.Response(200, json={"state": "running", "receipt_ref": None, "result_ref": None,
                                             "reason_code": None})
        return httpx.Response(200, json={"state": "completed", "receipt_ref": "rcpt-1", "result_ref": "res-1",
                                         "reason_code": None})

    def r_lab_results_id(self, m: Any, body: Any) -> httpx.Response:
        return httpx.Response(200, json={"columns": self.columns, "rows": self.rows,
                                         "truncated": self.truncated,
                                         "next_cursor": "c2" if self.truncated else None,
                                         "total_rows": self.total_rows, "receipt_ref": "rcpt-1",
                                         "result_digest": "e" * 64})

    def r_lab_sessions_id(self, m: Any, body: Any) -> httpx.Response:
        return httpx.Response(200, json={"state": "open", "revision": 3, "current_query_ref": None,
                                         "limits": {}})

    def r_lab_sessions_id_close(self, m: Any, body: Any) -> httpx.Response:
        return httpx.Response(200, json={"state": "closed"})

    def r_wiki_read(self, m: Any, body: dict[str, Any]) -> httpx.Response:
        return httpx.Response(200, json={"entries": [{"path": p, "content": "page " + p, "digest": "a" * 64,
                                                      "evidence_refs": []} for p in body["paths"]],
                                         "base_digest": "b" * 64})

    def r_wiki_explore(self, m: Any, body: dict[str, Any]) -> httpx.Response:
        return httpx.Response(200, json={"entries": [], "next_cursor": None, "truncated": False})

    def r_wiki_transform(self, m: Any, body: dict[str, Any]) -> httpx.Response:
        return httpx.Response(200, json={"scratch_ref": "scr-1", "diff_ref": "diff-1",
                                         "manifest_digest": "c" * 64, "violations": []})
