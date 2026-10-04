"""SNET spike probe (stdlib only). Host side: `serve` runs a shim with /time and /sleep?s=N. Container side:
`probe` measures reachability, clock skew, and calls of chosen durations against a client limit.

  host:       python snet_probe.py serve --host 0.0.0.0 --port 18080
  container:  python snet_probe.py probe --base http://host.containers.internal:18080 [--quick] [--long]

Exit code 1 when the shim is unreachable. No secrets, no model calls: the shim only sleeps."""

from __future__ import annotations

import argparse
import json
import socket
import sys
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse


class _Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802
        url = urlparse(self.path)
        if url.path == "/sleep":
            time.sleep(float(parse_qs(url.query).get("s", ["0"])[0]))
        elif url.path != "/time":
            self.send_error(404)
            return
        body = json.dumps({"t": time.time()}).encode()
        try:
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass  # the client already hit its limit

    def log_message(self, *args: object) -> None:
        pass


def make_server(host: str, port: int) -> ThreadingHTTPServer:
    server = ThreadingHTTPServer((host, port), _Handler)
    server.daemon_threads = True
    return server


def _get(url: str, timeout_s: float) -> dict:
    with urllib.request.urlopen(url, timeout=timeout_s) as resp:  # noqa: S310 (spike, operator-supplied URL)
        return json.loads(resp.read())


def probe_reachability(base: str, timeout_s: float = 5) -> dict:
    t0 = time.time()
    try:
        remote = _get(f"{base}/time", timeout_s)["t"]
    except (urllib.error.URLError, OSError, TimeoutError, ValueError) as exc:
        return {"reachable": False, "error": type(exc).__name__}
    t1 = time.time()
    # skew = remote clock minus local clock at the midpoint of the request
    return {"reachable": True, "rtt_s": round(t1 - t0, 4), "skew_s": round(remote - (t0 + t1) / 2, 4)}


def timed_call(base: str, sleep_s: float, limit_s: float) -> dict:
    t0 = time.monotonic()
    try:
        _get(f"{base}/sleep?s={sleep_s}", limit_s)
        outcome = "ok"
    except (socket.timeout, TimeoutError):
        outcome = "timeout"
    except urllib.error.URLError as exc:
        outcome = "timeout" if isinstance(exc.reason, (socket.timeout, TimeoutError)) else f"error:{exc.reason}"
    return {"sleep_s": sleep_s, "limit_s": limit_s, "outcome": outcome, "elapsed_s": round(time.monotonic() - t0, 2)}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("serve")
    s.add_argument("--host", default="0.0.0.0")
    s.add_argument("--port", type=int, default=18080)
    p = sub.add_parser("probe")
    p.add_argument("--base", required=True)
    p.add_argument("--quick", action="store_true", help="reachability and skew only")
    p.add_argument("--long", action="store_true", help="also a 300 s call under a 600 s limit")
    a = ap.parse_args(argv)
    if a.cmd == "serve":
        make_server(a.host, a.port).serve_forever()
        return 0
    report: dict = {"reachability": probe_reachability(a.base)}
    if not report["reachability"]["reachable"]:
        print(json.dumps(report))
        return 1
    if not a.quick:
        report["calls"] = [timed_call(a.base, 1, 60), timed_call(a.base, 70, 60), timed_call(a.base, 70, 600)]
        if a.long:
            report["calls"].append(timed_call(a.base, 300, 600))
    print(json.dumps(report, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
