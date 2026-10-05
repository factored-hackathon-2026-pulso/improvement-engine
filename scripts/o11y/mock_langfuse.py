"""MOCK Langfuse for offline tests and dry runs of langfuse_closure.ps1 (stdlib only; never a stand-in for the real thing).

Checks what Langfuse Cloud requires: Basic auth (pk:sk), `x-langfuse-ingestion-version: 4` on the OTLP route, OTLP JSON and
protobuf (optionally gzip). Serves the read-back routes the closure uses: /api/public/health, /api/public/traces[/{id}],
/api/public/observations, /api/public/scores, /api/public/models (GET, POST). `reject_protobuf=True` answers protobuf with 415
(a Langfuse that does not take protobuf) to test the diagnosis.

    python mock_langfuse.py --port 4399 --pk pk-lf-mock --sk sk-lf-mock
"""
from __future__ import annotations

import argparse
import base64
import json
import re
import struct
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import otlp_decode  # noqa: E402


# ---- protobuf encoder (tests) --------------------------------------------------------------------------------------
def _v(n: int) -> bytes:
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        out.append(b | (0x80 if n else 0))
        if not n:
            return bytes(out)


def _ld(f: int, b: bytes) -> bytes:
    return _v(f << 3 | 2) + _v(len(b)) + b


def _any(v) -> bytes:
    if isinstance(v, bool):
        return _v(2 << 3) + _v(int(v))
    if isinstance(v, int):
        return _v(3 << 3) + _v(v & ((1 << 64) - 1))
    if isinstance(v, float):
        return _v(4 << 3 | 1) + struct.pack("<d", v)
    if isinstance(v, list):
        return _ld(5, b"".join(_ld(1, _any(x)) for x in v))
    return _ld(1, str(v).encode())


def _kvb(k: str, v) -> bytes:
    return _ld(1, k.encode()) + _ld(2, _any(v))


def encode_protobuf(spans: list[dict]) -> bytes:
    """spans: dicts as otlp_decode produces (trace_id/span_id/parent_id hex, name, attrs, resource)."""
    by_res: dict[str, list[dict]] = {}
    for s in spans:
        by_res.setdefault(json.dumps(s.get("resource", {}), sort_keys=True), []).append(s)
    out = b""
    for res, group in by_res.items():
        r = b"".join(_ld(1, _kvb(k, v)) for k, v in json.loads(res).items())
        sp = b""
        for s in group:
            b = _ld(1, bytes.fromhex(s["trace_id"])) + _ld(2, bytes.fromhex(s["span_id"]))
            if s.get("parent_id"):
                b += _ld(4, bytes.fromhex(s["parent_id"]))
            b += _ld(5, s["name"].encode()) + b"".join(_ld(9, _kvb(k, v)) for k, v in s.get("attrs", {}).items())
            sp += _ld(2, b)
        out += _ld(1, _ld(1, r) + _ld(2, sp))
    return out


# ---- the mock ------------------------------------------------------------------------------------------------------
MODEL_ATTRS = ("langfuse.observation.model.name", "gen_ai.response.model", "gen_ai.request.model")


class MockLangfuse:
    def __init__(self, pk="pk-lf-mock", sk="sk-lf-mock", reject_protobuf=False, rate_limit_first=0):
        self.pk, self.sk, self.reject_protobuf = pk, sk, reject_protobuf
        self.auth = "Basic " + base64.b64encode(f"{pk}:{sk}".encode()).decode()
        self.spans: list[dict] = []
        self.scores: dict[str, dict] = {}
        self.models: dict[str, dict] = {}
        self.problems: list[str] = []
        self.hits = {"json": 0, "protobuf": 0}
        self.rate_limit = rate_limit_first
        self.lock = threading.Lock()
        self.httpd: ThreadingHTTPServer | None = None

    def _usage(self, a):
        raw = a.get("langfuse.observation.usage_details")
        if raw:
            try:
                d = json.loads(raw)
                return d.get("input", 0), d.get("output", 0)
            except ValueError:
                pass
        return a.get("gen_ai.usage.input_tokens", 0) or 0, a.get("gen_ai.usage.output_tokens", 0) or 0

    def observation(self, s: dict) -> dict:
        a = s["attrs"]
        model = next((a[k] for k in MODEL_ATTRS if a.get(k)), None)
        gen = a.get("langfuse.observation.type") == "generation" or model is not None
        tin, tout = self._usage(a)
        cost, model_id = None, None
        for m in self.models.values():
            if model and re.match(m["matchPattern"], model):
                cost, model_id = tin * m["inputPrice"] + tout * m["outputPrice"], m["id"]
        if cost is None and a.get("langfuse.observation.cost_details"):
            try:
                cost = json.loads(a["langfuse.observation.cost_details"]).get("total")
            except ValueError:
                pass
        return {"id": s["span_id"], "traceId": s["trace_id"], "parentObservationId": s["parent_id"] or None, "name": s["name"],
                "type": "GENERATION" if gen else "SPAN", "model": model, "modelId": model_id,
                "input": a.get("langfuse.observation.input") or a.get("gen_ai.prompt"),
                "output": a.get("langfuse.observation.output") or a.get("gen_ai.completion"),
                "calculatedTotalCost": cost, "usageDetails": {"input": tin, "output": tout},
                "metadata": {"resourceAttributes": s["resource"]}}

    def traces(self) -> dict[str, dict]:
        out: dict[str, dict] = {}
        for s in self.spans:
            t = out.setdefault(s["trace_id"], {"id": s["trace_id"], "name": None, "observations": [], "scores": []})
            t["observations"].append(s["span_id"])
            if not s["parent_id"] and not t["name"]:
                t["name"] = s["name"]
        for sc in self.scores.values():
            if sc["traceId"] in out:
                out[sc["traceId"]]["scores"].append(sc["id"])
        return out

    def start(self, port=0):
        mock = self

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _send(self, code, obj=None):
                body = json.dumps(obj if obj is not None else {}).encode()
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def _auth(self):
                if self.headers.get("Authorization") != mock.auth:
                    self._send(401, {"message": "Invalid credentials"})
                    return False
                return True

            def _page(self, items, q):
                limit, page = int(q.get("limit", 50)), int(q.get("page", 1))
                self._send(200, {"data": items[(page - 1) * limit:page * limit],
                                 "meta": {"page": page, "limit": limit, "totalItems": len(items),
                                          "totalPages": max(1, -(-len(items) // limit))}})

            def do_GET(self):  # noqa: N802
                u = urllib.parse.urlparse(self.path)
                q = {k: v[0] for k, v in urllib.parse.parse_qs(u.query).items()}
                if u.path == "/api/public/health":
                    return self._send(200, {"status": "OK", "version": "mock"})
                if not self._auth():
                    return
                with mock.lock:
                    if mock.rate_limit > 0:
                        mock.rate_limit -= 1
                        return self._send(429, {"message": "rate limited"})
                    if u.path == "/api/public/models":
                        return self._page(list(mock.models.values()), q)
                    if u.path == "/api/public/traces":
                        return self._page(list(mock.traces().values()), q)
                    m = re.fullmatch(r"/api/public/traces/([0-9a-f]+)", u.path)
                    if m:
                        t = mock.traces().get(m.group(1))
                        if not t:
                            return self._send(404)
                        return self._send(200, {**t, "observations": [mock.observation(s) for s in mock.spans if s["trace_id"] == t["id"]]})
                    if u.path == "/api/public/observations":
                        obs = [mock.observation(s) for s in mock.spans]
                        if q.get("traceId"):
                            obs = [o for o in obs if o["traceId"] == q["traceId"]]
                        if q.get("type"):
                            obs = [o for o in obs if o["type"] == q["type"]]
                        return self._page(obs, q)
                    if u.path == "/api/public/scores":
                        return self._page(list(mock.scores.values()), q)
                self._send(404)

            def do_POST(self):  # noqa: N802
                if not self._auth():
                    return
                raw = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                p = self.path
                with mock.lock:
                    if p == "/api/public/otel/v1/traces":
                        ct = (self.headers.get("Content-Type") or "").split(";")[0].strip()
                        if self.headers.get("x-langfuse-ingestion-version") != "4":
                            mock.problems.append("missing x-langfuse-ingestion-version: 4")
                        kind = "protobuf" if ct == "application/x-protobuf" else "json"
                        if kind == "protobuf" and mock.reject_protobuf:
                            return self._send(415, {"message": "unsupported media type"})
                        try:
                            spans = otlp_decode.decode(raw, ct)
                        except Exception:  # noqa: BLE001
                            mock.problems.append(f"undecodable {kind} body")
                            return self._send(400)
                        mock.hits[kind] += 1
                        mock.spans.extend(spans)
                        return self._send(200, {})
                    if p == "/api/public/scores":
                        sc = json.loads(raw)
                        sid = sc.get("id") or str(len(mock.scores))
                        mock.scores[sid] = {**sc, "id": sid}
                        return self._send(200, {"id": sid})
                    if p == "/api/public/models":
                        m = json.loads(raw)
                        if m["modelName"] in mock.models:
                            return self._send(400, {"message": "exists"})
                        mock.models[m["modelName"]] = {**m, "id": "m" + str(len(mock.models) + 1)}
                        return self._send(200, mock.models[m["modelName"]])
                self._send(404)

        self.httpd = ThreadingHTTPServer(("127.0.0.1", port), H)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        return self.httpd.server_address[1]

    def stop(self):
        if self.httpd:
            self.httpd.shutdown()
            self.httpd.server_close()


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--port", type=int, default=4399)
    ap.add_argument("--pk", default="pk-lf-mock")
    ap.add_argument("--sk", default="sk-lf-mock")
    ap.add_argument("--reject-protobuf", action="store_true")
    ap.add_argument("--dump", help="write the received spans (JSON) to this file on every request")
    a = ap.parse_args(argv)
    m = MockLangfuse(a.pk, a.sk, a.reject_protobuf)
    port = m.start(a.port)
    print(f"mock langfuse on 127.0.0.1:{port}", flush=True)
    import time
    while True:
        time.sleep(1)
        if a.dump:
            with m.lock:
                Path(a.dump).write_text(json.dumps({"spans": m.spans, "scores": list(m.scores.values()), "hits": m.hits,
                                                    "problems": m.problems}), encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
