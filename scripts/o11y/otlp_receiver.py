"""Tiny LOCAL OTLP/HTTP JSON receiver (stdlib) that VALIDATES the trace shape and stores a one-line summary per span.

    python otlp_receiver.py --port 4318 --out spans.jsonl

Accepts `POST /v1/traces` with `{"resourceSpans": [...]}`; replies 400 with the reason when a span is malformed
(32-hex traceId, 16-hex spanId, nanosecond start <= end, typed attribute values). Binds to 127.0.0.1 only.
"""
from __future__ import annotations

import argparse
import json
import re
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

HEX32, HEX16 = re.compile(r"^[0-9a-f]{32}$"), re.compile(r"^[0-9a-f]{16}$")
VALUE_KEYS = {"stringValue", "intValue", "doubleValue", "boolValue", "arrayValue"}


def validate(doc: dict) -> list[dict]:
    """Return one summary dict per span or raise ValueError with the first problem found."""
    out = []
    if not isinstance(doc.get("resourceSpans"), list) or not doc["resourceSpans"]:
        raise ValueError("resourceSpans must be a non-empty list")
    for rs in doc["resourceSpans"]:
        for ss in rs.get("scopeSpans", []):
            for sp in ss.get("spans", []):
                if not HEX32.match(sp.get("traceId", "")) or not HEX16.match(sp.get("spanId", "")):
                    raise ValueError("bad traceId/spanId")
                if "parentSpanId" in sp and not HEX16.match(sp["parentSpanId"]):
                    raise ValueError("bad parentSpanId")
                s, e = int(sp["startTimeUnixNano"]), int(sp["endTimeUnixNano"])
                if not (0 < s <= e):
                    raise ValueError("bad span times")
                attrs = {}
                for a in sp.get("attributes", []):
                    if not a.get("key") or len(a.get("value", {})) != 1 or not set(a["value"]) <= VALUE_KEYS:
                        raise ValueError("bad attribute " + str(a.get("key")))
                    attrs[a["key"]] = next(iter(a["value"].values()))
                out.append({"trace": sp["traceId"], "span": sp["spanId"], "parent": sp.get("parentSpanId"),
                            "name": sp["name"], "ms": (e - s) // 1_000_000, "attrs": attrs})
    return out


def make_server(port: int, out: Path | None = None) -> HTTPServer:
    class H(BaseHTTPRequestHandler):
        def do_POST(self):  # noqa: N802
            body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
            try:
                if self.path != "/v1/traces":
                    raise ValueError("path must be /v1/traces")
                spans = validate(json.loads(body))
            except (ValueError, KeyError, TypeError) as e:
                self.send_response(400)
                self.end_headers()
                self.wfile.write(str(e).encode())
                return
            if out:
                with out.open("a", encoding="utf-8") as f:
                    for s in spans:
                        f.write(json.dumps(s, sort_keys=True) + "\n")
            self.server.received.extend(spans)  # type: ignore[attr-defined]
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b"{}")

        def log_message(self, *a):
            pass

    srv = HTTPServer(("127.0.0.1", port), H)
    srv.received = []  # type: ignore[attr-defined]
    return srv


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=4318)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    make_server(a.port, a.out).serve_forever()
