"""Real-HTTP loopback double of the control-api binding route and the lab-broker (annex D.3), reusing the
route-table-driven `FakeBackend` of the L3b tests. It is ALWAYS a double: the platform side (Codex) is not part
of this repository's runtime."""

from __future__ import annotations

import json
import re
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

import httpx
from l3b.d3_fixture import FakeBackend


SANDBOX = "/internal/v1/broker/sandbox"
SANDBOX_ROUTES: tuple[tuple[str, str, tuple[str, ...]], ...] = (  # plan D.4 (bank), verbatim
    ("POST", r"/sessions", ("campaign_ref", "case_ref", "arm", "repetition", "seed_manifest_ref")),
    ("POST", r"/sessions/([^/]+)/reset", ()),
    ("POST", r"/sessions/([^/]+)/actions", ("action_key", "expected_revision", "action")),
    ("GET", r"/sessions/([^/]+)/actions/([^/]+)", ()),
    ("POST", r"/sessions/([^/]+)/close", ("reason",)))


class BankBackend(FakeBackend):
    """Double of the Codex stateful bank (`/internal/v1/broker/sandbox`): revisioned sessions, idempotent actions
    (same key + payload replays the receipt, same key + other payload is 409), readback by key, close evidence."""

    def __init__(self) -> None:
        super().__init__()
        self.sessions: dict[str, dict[str, Any]] = {}
        self.act_mode = "ok"  # ok | lose_response (effect applied, answer lost) | refuse
        self.open_mode = "ok"  # ok | 503

    def _bind(self, request: httpx.Request, body: dict[str, Any]) -> httpx.Response:
        if self.bind_mode == "applied_then_503":  # the platform recorded the binding, the answer is a 5xx
            self.bound.append((body["tenant_id"], body["job_id"]))
            return httpx.Response(503, json={"code": "unavailable"})
        return super()._bind(request, body)

    def handle(self, request: httpx.Request) -> httpx.Response:
        if not request.url.path.startswith(SANDBOX):
            return super().handle(request)
        self.requests.append(request)
        body = json.loads(request.content) if request.content else None
        self.bodies.append(body)
        suffix = request.url.path[len(SANDBOX):]
        for method, pattern, keys in SANDBOX_ROUTES:
            m = re.fullmatch(pattern, suffix)
            if m and method == request.method:
                if body is not None and not set(body) <= set(keys):
                    return httpx.Response(400, json={"code": "unknown_field"})
                return self._route(pattern, m, body, request)
        return httpx.Response(404, json={"code": "not_found"})

    def _route(self, pattern: str, m: re.Match[str], body: Any, request: httpx.Request) -> httpx.Response:
        if pattern == r"/sessions":
            if self.open_mode == "503":
                return httpx.Response(503, json={"code": "unavailable"})
            ref = f"bank-{len(self.sessions) + 1}"
            self.sessions[ref] = {"revision": 0, "actions": {}, "open": body, "closed": None}
            return httpx.Response(200, json={"session_ref": ref, "revision": 0,
                                             "initial_state_digest": "sha256:" + "1" * 64})
        sess = self.sessions.get(m.group(1))
        if sess is None:
            return httpx.Response(404, json={"code": "session_not_found"})
        if pattern.endswith("/reset"):
            sess["revision"] = 0
            sess["actions"].clear()
            return httpx.Response(200, json={"session_ref": m.group(1), "revision": 0,
                                             "initial_state_digest": "sha256:" + "1" * 64})
        if pattern.endswith("/close"):
            sess["closed"] = body["reason"]
            return httpx.Response(200, json={"state": "closed", "final_state_ref": f"final:{m.group(1)}"})
        if request.method == "GET":
            got = sess["actions"].get(m.group(2))
            return httpx.Response(404, json={"code": "action_not_found"}) if got is None                 else httpx.Response(200, json=got["receipt"])
        key = body["action_key"]
        prior = sess["actions"].get(key)
        if prior is not None:
            if prior["payload"] != body["action"]:
                return httpx.Response(409, json={"code": "action_key_conflict"})
            return httpx.Response(200, json=prior["receipt"])
        if self.act_mode == "refuse":
            return httpx.Response(403, json={"code": "forbidden"})
        if body["expected_revision"] != sess["revision"]:
            return httpx.Response(409, json={"code": "revision_conflict"})
        sess["revision"] += 1
        receipt = {"revision": sess["revision"], "effect_receipt": f"rcpt-{key[:8]}",
                   "result": {"ok": True, "type": body["action"]["type"]}, "state_digest": "sha256:" + "2" * 64}
        sess["actions"][key] = {"payload": body["action"], "receipt": receipt}
        if self.act_mode == "lose_response":
            return httpx.Response(504, json={"code": "gateway_timeout"})  # applied, answer lost
        return httpx.Response(200, json=receipt)


class Loopback:
    def __init__(self, backend: FakeBackend | None = None) -> None:
        self.backend = backend or FakeBackend()
        backend = self.backend

        class Handler(BaseHTTPRequestHandler):
            def _serve(self) -> None:
                length = int(self.headers.get("content-length") or 0)
                body = self.rfile.read(length) if length else b""
                request = httpx.Request(self.command, "http://loopback" + self.path,
                                        headers=dict(self.headers.items()), content=body)
                try:
                    resp = backend.handle(request)
                except httpx.HTTPError:
                    self.send_response(504)
                    self.end_headers()
                    return
                self.send_response(resp.status_code)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(resp.content)))
                self.end_headers()
                self.wfile.write(resp.content)

            do_GET = do_POST = _serve

            def log_message(self, *args: Any) -> None:  # silence
                return

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self._server.server_address[1]}"
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    @property
    def bindings(self) -> list[dict[str, Any]]:
        return [b for r, b in zip(self.backend.requests, self.backend.bodies, strict=True)
                if r.url.path == "/internal/v1/core-task-bindings" and b]

    def close(self) -> None:
        self._server.shutdown()
        self._server.server_close()
