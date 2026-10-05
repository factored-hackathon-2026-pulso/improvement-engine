"""LOCAL OTLP/HTTP forwarder (O11Y1): the single egress to Langfuse for services that cannot do TLS or hold keys.

Listens on 127.0.0.1 only. `POST /v1/traces` (OTLP JSON) is masked (bearer tokens, sk-/pk-lf- keys, JWTs, key=value
secrets in every string), queued, batched and forwarded to LANGFUSE_BASE_URL/api/public/otel/v1/traces with Basic
auth (public:secret) and `x-langfuse-ingestion-version: 4`. `POST /v1/scores` goes to /api/public/scores. Retries with
exponential backoff on 429/5xx/network errors (honours Retry-After); minimum intervals keep ingestion under
1000 req/min and scores under 30 req/min. Credentials come from the process environment only and are never printed or
logged. A non-loopback upstream requires https and --allow-external / PULSO_O11Y_ALLOW_EXTERNAL=1.

    python otlp_forwarder.py [--port 4318] [--env-file F] [--allow-external]
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import queue
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from runtrace_bridge import LOOPBACK, SECRET_RE, _load_env_file, _truthy  # noqa: E402

TRACE_INTERVAL = 60 / 900   # under 1000 req/min
SCORE_INTERVAL = 60 / 25    # under 30 req/min


def mask_value(v):
    if isinstance(v, str):
        return SECRET_RE.sub("[redacted]", v)
    if isinstance(v, list):
        return [mask_value(x) for x in v]
    if isinstance(v, dict):
        return {k: mask_value(x) for k, x in v.items()}
    return v


def merge_batches(bodies: list[dict]) -> dict:
    return {"resourceSpans": [rs for b in bodies for rs in b["resourceSpans"]]}


class Forwarder:
    def __init__(self, env: dict, allow_external: bool, max_retries: int = 6, backoff: float = 1.0,
                 sleep=time.sleep, batch_max: int = 50):
        base, pk, sk = env.get("LANGFUSE_BASE_URL", ""), env.get("LANGFUSE_PUBLIC_KEY", ""), env.get("LANGFUSE_SECRET_KEY", "")
        if not (base and pk and sk):
            raise ValueError("needs LANGFUSE_BASE_URL, LANGFUSE_PUBLIC_KEY and LANGFUSE_SECRET_KEY in the environment")
        u = urllib.parse.urlparse(base)
        if not u.hostname or u.scheme not in ("http", "https"):
            raise ValueError("invalid LANGFUSE_BASE_URL")
        if u.hostname not in LOOPBACK:
            if not allow_external:
                raise ValueError("external forwarding needs --allow-external or PULSO_O11Y_ALLOW_EXTERNAL=1")
            if u.scheme != "https":
                raise ValueError("external forwarding requires https")
        auth = base64.b64encode(f"{pk}:{sk}".encode()).decode()
        self.root = f"{u.scheme}://{u.netloc}"
        self.auth, self.secrets = auth, [pk, sk, auth]
        self.max_retries, self.backoff, self.sleep, self.batch_max = max_retries, backoff, sleep, batch_max
        self.q: queue.Queue = queue.Queue()
        self.stats = {"forwarded": 0, "failed": 0, "retries": 0}
        self._last = {"traces": 0.0, "scores": 0.0}

    def post(self, kind: str, body: dict) -> None:
        path, gap = (("/api/public/otel/v1/traces", TRACE_INTERVAL) if kind == "traces"
                     else ("/api/public/scores", SCORE_INTERVAL))
        headers = {"Authorization": f"Basic {self.auth}", "Content-Type": "application/json"}
        if kind == "traces":
            headers["x-langfuse-ingestion-version"] = "4"
        data = json.dumps(body, separators=(",", ":")).encode()
        for attempt in range(self.max_retries + 1):
            wait = gap - (time.monotonic() - self._last[kind])
            if wait > 0:
                self.sleep(wait)
            self._last[kind] = time.monotonic()
            try:
                urllib.request.urlopen(urllib.request.Request(self.root + path, data=data, headers=headers,
                                                              method="POST"), timeout=30).close()
                self.stats["forwarded"] += 1
                return
            except urllib.error.HTTPError as e:
                if e.code not in (408, 429) and e.code < 500:
                    self.stats["failed"] += 1
                    raise RuntimeError(f"upstream HTTP {e.code}") from None
                ra = e.headers.get("Retry-After") if e.headers else None
                delay = float(ra) if ra and ra.replace(".", "", 1).isdigit() else self.backoff * 2 ** attempt
            except OSError:
                delay = self.backoff * 2 ** attempt
            self.stats["retries"] += 1
            self.sleep(min(delay, 60))
        self.stats["failed"] += 1
        raise RuntimeError("upstream unavailable after retries")

    def enqueue(self, kind: str, body: dict) -> None:
        self.q.put((kind, mask_value(body)))

    def drain_once(self) -> int:
        """Take what is queued, merge trace batches, forward. Returns items handled."""
        items = []
        try:
            while len(items) < self.batch_max:
                items.append(self.q.get_nowait())
        except queue.Empty:
            pass
        traces = [b for k, b in items if k == "traces"]
        if traces:
            self._safe("traces", merge_batches(traces))
        for k, b in items:
            if k == "scores":
                self._safe("scores", b)
        return len(items)

    def _safe(self, kind, body):
        try:
            self.post(kind, body)
        except RuntimeError as e:
            print(f"forwarder: {kind} dropped: {e}", file=sys.stderr)

    def run_worker(self, stop: threading.Event) -> None:
        while not stop.is_set():
            if not self.drain_once():
                stop.wait(0.2)


def make_server(fwd: Forwarder, port: int) -> ThreadingHTTPServer:
    class H(BaseHTTPRequestHandler):
        def do_POST(self):  # noqa: N802
            kind = {"/v1/traces": "traces", "/v1/scores": "scores"}.get(self.path)
            try:
                if kind is None:
                    raise ValueError("path")
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
                if kind == "traces" and not isinstance(body.get("resourceSpans"), list):
                    raise ValueError("resourceSpans")
            except (ValueError, TypeError):
                self.send_response(400)
                self.end_headers()
                return
            fwd.enqueue(kind, body)
            self.send_response(202)
            self.end_headers()
            self.wfile.write(b"{}")

        def log_message(self, *a):
            pass

    return ThreadingHTTPServer(("127.0.0.1", port), H)  # loopback listener only


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--port", type=int, default=4318)
    ap.add_argument("--env-file", action="append", default=[])
    ap.add_argument("--allow-external", action="store_true")
    a = ap.parse_args(argv)
    env = dict(os.environ)
    for f in a.env_file:
        env.update(_load_env_file(f))
    try:
        fwd = Forwarder(env, a.allow_external or _truthy(env.get("PULSO_O11Y_ALLOW_EXTERNAL")))
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    stop = threading.Event()
    threading.Thread(target=fwd.run_worker, args=(stop,), daemon=True).start()
    print(f"forwarder listening on 127.0.0.1:{a.port} -> {urllib.parse.urlparse(fwd.root).hostname}")
    try:
        make_server(fwd, a.port).serve_forever()
    except KeyboardInterrupt:
        stop.set()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
