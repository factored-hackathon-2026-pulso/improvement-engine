"""ANN1 test double of the support-platform announce route (PR 17, ADR 0007). Hermetic (stdlib only); mirrors the contract in
docs/platform/api/improvement-announce.md: Bearer service token, bounded camelCase body, unknown fields rejected, no email / 9+ digit
runs, <= 8 distinct CASE ids, origin auto_detect only, idempotent per proposal id (one notification per supervisor).
It is a DOUBLE: the real platform is exercised by serve_platform_live.py.
"""
from __future__ import annotations

import hmac
import json
import re
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROUTE = "/api/v1/internal/builder/proposals/announce"
EMAIL = re.compile(r"[^\s@]+@[^\s@]+\.[A-Za-z]{2,}")
LONG_NUMBER = re.compile(r"(?:\d[ \-.]?){9,}")
CASE = re.compile(r"^CASE-[0123456789ABCDEFGHJKMNPQRSTVWXYZ]{26}$")
LIMITS = {"title": 120, "problem": 600, "evidence": 600, "expectedEffect": 400}
FIELDS = {"proposalId", *LIMITS, "evidenceLinks"}


def validate(body: object) -> str | None:
    if not isinstance(body, dict) or set(body) - FIELDS or not {"proposalId", *LIMITS} <= set(body):
        return "fields"
    pid = body["proposalId"]
    if not isinstance(pid, str) or not 1 <= len(pid) <= 64:
        return "proposalId"
    for name, cap in LIMITS.items():
        v = body[name]
        if not isinstance(v, str) or not 1 <= len(v.strip()) <= cap:
            return name
        if EMAIL.search(v) or LONG_NUMBER.search(v):
            return f"{name}:personal_data"
    links = body.get("evidenceLinks", [])
    if not isinstance(links, list) or len(links) > 8 or len(set(map(str, links))) != len(links) or not all(isinstance(x, str) and CASE.match(x) for x in links):
        return "evidenceLinks"
    return None


class Platform:
    """State: known proposals {id: origin}, adopted ids, notifications {id: supervisors notified}."""

    def __init__(self, token: str, proposals: dict[str, str], supervisors: int = 2) -> None:
        self.token, self.proposals, self.supervisors = token, dict(proposals), supervisors
        self.adopted: dict[str, dict] = {}
        self.notifications: dict[str, int] = {}
        self.calls = 0
        self.lock = threading.Lock()

    def announce(self, authorization: str | None, body: object) -> tuple[int, dict]:
        with self.lock:
            self.calls += 1
            if not hmac.compare_digest((authorization or "").removeprefix("Bearer ").strip().encode(), self.token.encode()):
                return 401, {"code": "authentication_required"}
            bad = validate(body)
            if bad:
                return 422, {"code": "validation_error", "detail": bad}
            pid = body["proposalId"]  # type: ignore[index]
            if pid not in self.proposals:
                return 404, {"code": "not_found"}
            if self.proposals[pid] != "auto_detect":
                return 422, {"code": "validation_error", "detail": "origin"}
            self.adopted.setdefault(pid, {"proposalId": pid, "origin": "auto_detect", "state": "draft", "source": "engine", "registeredBy": "engine"})
            self.notifications.setdefault(pid, self.supervisors)
            return 200, self.adopted[pid]


def serve(platform: Platform, port: int = 0) -> ThreadingHTTPServer:
    class H(BaseHTTPRequestHandler):
        def do_POST(self) -> None:  # noqa: N802
            raw = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            if self.path != ROUTE:
                code, out = 404, {"code": "not_found"}
            else:
                try:
                    body = json.loads(raw)
                except ValueError:
                    body = None
                code, out = platform.announce(self.headers.get("Authorization"), body)
            data = json.dumps(out).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *a: object) -> None:
            pass

    srv = ThreadingHTTPServer(("127.0.0.1", port), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv
