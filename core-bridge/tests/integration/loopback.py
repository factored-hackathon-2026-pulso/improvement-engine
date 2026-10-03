"""Real-HTTP loopback double of the control-api binding route and the lab-broker (annex D.3), reusing the
route-table-driven `FakeBackend` of the L3b tests. It is ALWAYS a double: the platform side (Codex) is not part
of this repository's runtime."""

from __future__ import annotations

import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

import httpx
from l3b.d3_fixture import FakeBackend


class Loopback:
    def __init__(self) -> None:
        self.backend = FakeBackend()
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
