"""Two PLAIN agent runs through agent-core `POST /v1/runs` (disputas in Spanish, consultas in Portuguese), each with its own
trace id and a `traceparent`, so agent-core (and the gateway behind it) export spans under that trace. A small engine-side root
span `pulso.agent_run` (with the user text and the replies) is sent through the local forwarder so the trace has a root.

SYNTHETIC input only, local demo auth (agent-core's test issuer, valid only against the local dev stack).

    python agent_runs.py --base http://127.0.0.1:8191 --forwarder http://127.0.0.1:4318 --out agent_runs.json
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import runtrace_bridge as rb  # noqa: E402
from trace_id import build_traceparent, root_span_id, story_trace_id  # noqa: E402

SCENARIOS = [
    {"key": "disputas-es", "agent": "disputas", "lang": "es", "pid": "synth-lfc-es",
     "text": "no reconozco un cargo de 120 dolares en una tienda en linea"},
    {"key": "consultas-pt", "agent": "consultas", "lang": "pt", "pid": "synth-lfc-pt",
     "text": "qual e o status da minha solicitacao de segunda via do cartao?"},
]


def call(base: str, method: str, path: str, token: str, body=None, headers=None, timeout=180):
    req = urllib.request.Request(base.rstrip("/") + path, method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json", "Authorization": "Bearer " + token, **(headers or {})})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b"{}")
        except ValueError:
            return e.code, {}
    except OSError as e:
        return 0, {"code": type(e).__name__}


def play(base: str, issuer, s: dict, tp: str) -> dict:
    t0 = time.time()
    h = {"traceparent": tp}
    tok = issuer.customer(s["pid"])
    st, body = call(base, "POST", "/v1/runs", tok, {"agent": s["agent"], "lang": s["lang"]}, {**h, "Idempotency-Key": "lfc-" + uuid.uuid4().hex})
    out = {"start_http": st, "replies": [], "outcome": None, "status": None}
    if st != 201:
        out["error"] = f"start http {st} {body.get('code', '')}"
        return out
    sid = body["session_id"]
    turn = body.get("first_turn") or {}
    out["replies"].append(str(turn.get("reply") or turn.get("text") or ""))
    code, turn = call(base, "POST", f"/v1/sessions/{sid}/turns", tok,
                      {"channel": "web", "client_turn_id": "t-" + uuid.uuid4().hex, "text": s["text"]}, h)
    out["turn_http"] = code
    if code == 200:
        out["replies"].append(str(turn.get("reply") or turn.get("text") or ""))
        if turn.get("confirmation"):
            tok2 = issuer.stepped_up(s["pid"])
            code, turn = call(base, "POST", f"/v1/sessions/{sid}/turns", tok2,
                              {"channel": "web", "client_turn_id": "t-" + uuid.uuid4().hex,
                               "confirm": {"token": turn["confirmation"]["token"], "answer": "yes"}}, h)
            out["confirm_http"] = code
            if code == 200:
                out["replies"].append(str(turn.get("reply") or turn.get("text") or ""))
        out["status"], out["outcome"] = turn.get("status"), turn.get("outcome")
    out["duration_ms"] = int((time.time() - t0) * 1000)
    return out


def root_span(trace_id: str, s: dict, res: dict, start_ns: int, end_ns: int) -> dict:
    scrub = lambda v: rb.SECRET_RE.sub("[redacted]", v)[:20000]  # noqa: E731
    attrs = {"langfuse.trace.name": f"agent run {s['agent']} {s['lang']}", "langfuse.session.id": f"lfc-{s['key']}",
             "langfuse.trace.tags": ["pulso", "agent-run", f"agent:{s['agent']}", f"locale:{s['lang']}", "synthetic"],
             "langfuse.observation.type": "agent", "langfuse.observation.input": scrub(s["text"]),
             "langfuse.observation.output": scrub(json.dumps(res["replies"], ensure_ascii=False)),
             "pulso.outcome": res.get("outcome") or res.get("error") or "unknown", "pulso.synthetic": True}
    return {"resource": {"attributes": rb._attrs({"service.name": "pulso-engine"})},
            "scopeSpans": [{"scope": {"name": "pulso.agent-runs", "version": "1"}, "spans": [{
                "traceId": trace_id, "spanId": root_span_id(trace_id), "name": "pulso.agent_run", "kind": 1,
                "startTimeUnixNano": str(start_ns), "endTimeUnixNano": str(end_ns), "attributes": rb._attrs(attrs),
                "status": {"code": 1 if not res.get("error") else 2}}]}]}


def send_json(forwarder: str, rs: list[dict]) -> None:
    rb.require_loopback(forwarder)
    req = urllib.request.Request(forwarder.rstrip("/") + "/v1/traces", method="POST", data=json.dumps({"resourceSpans": rs}).encode(),
                                 headers={"Content-Type": "application/json"})
    urllib.request.urlopen(req, timeout=20).close()


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--base", default="http://127.0.0.1:8191")
    ap.add_argument("--forwarder", default="http://127.0.0.1:4318")
    ap.add_argument("--out", required=True)
    ap.add_argument("--agent-core-dir", default=os.environ.get("PULSO_AGENT_CORE_DIR", ""))
    ap.add_argument("--no-send", action="store_true", help="do not send the root spans (tests)")
    a = ap.parse_args(argv)
    try:
        from testing.fakes.identity import TestIdentityIssuer  # noqa: F401
    except ImportError:
        if os.environ.get("AGENT_RUNS_REEXEC") or not a.agent_core_dir or not Path(a.agent_core_dir).exists():
            print("error: local demo auth needs agent-core on the path (--agent-core-dir)", file=sys.stderr)
            return 1
        return subprocess.run(["uv", "run", "--project", a.agent_core_dir, "python", str(Path(__file__).resolve()), *(argv or sys.argv[1:])],
                              env={**os.environ, "AGENT_RUNS_REEXEC": "1"}).returncode
    from agent_core.adapters.system_clock import SystemClock
    from testing.fakes.identity import TestIdentityIssuer
    issuer_raw = TestIdentityIssuer(SystemClock())

    class Issuer:
        def customer(self, pid):
            return issuer_raw.customer(pid)

        def stepped_up(self, pid):
            return issuer_raw.stepped_up(pid)

    run_label = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S") + "-" + uuid.uuid4().hex[:6]
    manifest = {"agent_runs": [], "details": []}
    spans = []
    for s in SCENARIOS:
        tid = story_trace_id(f"agent-run:{s['key']}", run_label)
        tp = build_traceparent(tid, root_span_id(tid))
        start = time.time_ns()
        res = play(a.base, Issuer(), s, tp)
        spans.append(root_span(tid, s, res, start, time.time_ns()))
        manifest["agent_runs"].append(tid)
        manifest["details"].append({"key": s["key"], "trace_id": tid, **{k: v for k, v in res.items() if k != "replies"}})
        print(f"agent run {s['key']}: start_http={res.get('start_http')} status={res.get('status')} outcome={res.get('outcome') or res.get('error')} trace_id={tid}")
    if not a.no_send:
        send_json(a.forwarder, spans)
    Path(a.out).write_text(json.dumps(manifest, indent=1), encoding="utf-8")
    bad = [d["key"] for d in manifest["details"] if d.get("start_http") != 201]
    if bad:
        print(f"error: runs that did not start: {', '.join(bad)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
