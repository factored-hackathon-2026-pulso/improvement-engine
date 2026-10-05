"""Langfuse closure helpers (stdlib only): health ping, idempotent model registration, and READ-BACK verification.

    python langfuse_verify.py health   --env-file F            # prints only the status
    python langfuse_verify.py models   --env-file F --allow-external
    python langfuse_verify.py verify   --env-file F --allow-external --manifest traffic.json [--wait-secs 180]

Credentials come from the environment or --env-file (this process only), go in an Authorization header and are never printed.
Output is COUNTS ONLY. `verify` exits 1 with a precise diagnosis when something is missing.

Sources of a trace are told apart by observation name (and `service.name` in the observation metadata when Langfuse exposes it):
  engine      pulso.story, stage.*, generation <role>, pulso.agent_run
  gateway     chat <model>
  agent-core  agentcore.*, invoke_agent*, execute_tool*
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from runtrace_bridge import LOOPBACK, PRICES, _load_env_file, _truthy, redact  # noqa: E402

SOURCES = ("engine", "gateway", "agent-core")


class Client:
    """Basic-auth GET/POST client for the Langfuse public API. Throttled (Hobby: 30 requests/min), retries 429/5xx."""

    def __init__(self, base: str, pk: str, sk: str, allow_external=False, min_interval=2.1, sleep=time.sleep, retries=5):
        u = urllib.parse.urlparse(base)
        if not u.hostname or u.scheme not in ("http", "https"):
            raise ValueError("invalid LANGFUSE_BASE_URL")
        if u.hostname not in LOOPBACK:
            if not allow_external:
                raise ValueError("external host needs --allow-external or PULSO_O11Y_ALLOW_EXTERNAL=1")
            if u.scheme != "https":
                raise ValueError("external host requires https")
        self.root = f"{u.scheme}://{u.netloc}"
        auth = base64.b64encode(f"{pk}:{sk}".encode()).decode()
        self.headers = {"Authorization": f"Basic {auth}"}
        self.secrets = [pk, sk, auth]
        self.min_interval, self.sleep, self.retries, self._last = min_interval, sleep, retries, 0.0

    def request(self, method: str, path: str, params: dict | None = None, body: dict | None = None) -> dict:
        url = self.root + path + ("?" + urllib.parse.urlencode(params) if params else "")
        for attempt in range(self.retries + 1):
            wait = self.min_interval - (time.monotonic() - self._last)
            if wait > 0:
                self.sleep(wait)
            self._last = time.monotonic()
            req = urllib.request.Request(url, method=method, data=None if body is None else json.dumps(body).encode(),
                                         headers={"Content-Type": "application/json", **self.headers})
            try:
                with urllib.request.urlopen(req, timeout=30) as r:  # noqa: S310 (host policy enforced above)
                    raw = r.read()
                    return json.loads(raw) if raw else {}
            except urllib.error.HTTPError as e:
                if e.code in (429, 500, 502, 503, 504) and attempt < self.retries:
                    ra = e.headers.get("Retry-After") if e.headers else None
                    self.sleep(float(ra) if ra and ra.replace(".", "", 1).isdigit() else min(2 ** attempt * 2, 30))
                    continue
                raise RuntimeError(f"HTTP {e.code} on {method} {path}") from None
            except OSError as e:
                if attempt < self.retries:
                    self.sleep(min(2 ** attempt * 2, 30))
                    continue
                raise RuntimeError(redact(f"unreachable on {method} {path}: {type(e).__name__}", self.secrets)) from None
        raise RuntimeError(f"gave up on {method} {path}")

    def pages(self, path: str, params: dict | None = None, limit=100):
        page = 1
        while True:
            d = self.request("GET", path, {**(params or {}), "limit": limit, "page": page})
            yield from d.get("data", [])
            if page >= int((d.get("meta") or {}).get("totalPages", 1)):
                return
            page += 1


def client_from_env(env: dict, allow_external: bool, **kw) -> Client:
    base, pk, sk = env.get("LANGFUSE_BASE_URL", ""), env.get("LANGFUSE_PUBLIC_KEY", ""), env.get("LANGFUSE_SECRET_KEY", "")
    missing = [k for k, v in (("LANGFUSE_BASE_URL", base), ("LANGFUSE_PUBLIC_KEY", pk), ("LANGFUSE_SECRET_KEY", sk)) if not v]
    if missing:
        raise ValueError("missing in the environment: " + ", ".join(missing))
    return Client(base, pk, sk, allow_external, **kw)


def health(client: Client) -> str:
    """Status line only. /api/public/health needs no auth."""
    req = urllib.request.Request(client.root + "/api/public/health")
    try:
        with urllib.request.urlopen(req, timeout=20) as r:  # noqa: S310
            return f"HTTP {r.status} {json.loads(r.read() or b'{}').get('status', '?')}"
    except urllib.error.HTTPError as e:
        return f"HTTP {e.code}"
    except OSError as e:
        return f"unreachable ({type(e).__name__})"


def ensure_models(client: Client, prices: dict | None = None) -> dict:
    """Idempotent: create the models that do not exist (by modelName), leave existing ones alone."""
    prices = prices or PRICES
    have = {m.get("modelName") for m in client.pages("/api/public/models")}
    made = []
    for name, (pin, pout) in prices.items():
        if name in have:
            continue
        client.request("POST", "/api/public/models", body={
            "modelName": name, "matchPattern": "(?i)^" + re.escape(name) + "$", "unit": "TOKENS",
            "inputPrice": pin, "outputPrice": pout})
        made.append(name)
    return {"created": made, "existing": sorted(have & set(prices))}


# ---------------------------------------------------------------------------------------------- classification

def source_of(obs: dict) -> str | None:
    name = str(obs.get("name") or "")
    svc = ((obs.get("metadata") or {}).get("resourceAttributes") or {})
    svc = str(svc.get("service.name") or (obs.get("metadata") or {}).get("service.name") or "")
    if name == "pulso.story" or name == "pulso.agent_run" or name.startswith(("stage.", "generation ")):
        return "engine"
    if name.startswith("chat ") or svc == "llm-gateway":
        return "gateway"
    if name.startswith(("agentcore.", "invoke_agent", "execute_tool")) or svc == "agentcore":
        return "agent-core"
    return None


def _model(o: dict):
    return o.get("model") or o.get("providedModelName")


def _cost(o: dict):
    c = o.get("calculatedTotalCost")
    if c is None:
        c = o.get("totalCost")
    return c


def _has(v) -> bool:
    return v not in (None, "", [], {})


def summarize(traces: dict[str, list[dict]], story_ids: set[str], scores: list[dict], expected_scores_min: int) -> dict:
    """Pure: observations per trace id -> counts and problems. `story_ids` are the traces that must show all three sources."""
    obs = [o for lst in traces.values() for o in lst]
    gens = [o for o in obs if str(o.get("type", "")).upper() == "GENERATION"]
    by_src = {s: 0 for s in SOURCES}
    for o in obs:
        s = source_of(o)
        if s:
            by_src[s] += 1
    all_three = [t for t in story_ids if {source_of(o) for o in traces.get(t, [])} >= set(SOURCES)]
    names = sorted({str(s.get("name")) for s in scores})
    rep = {
        "traces": len(traces), "observations": len(obs),
        "spans": sum(1 for o in obs if str(o.get("type", "")).upper() == "SPAN"), "generations": len(gens),
        "generations_with_content": sum(1 for o in gens if _has(o.get("input")) and _has(o.get("output"))),
        "observations_by_source": by_src,
        "story_traces": len(story_ids), "story_traces_with_all_three_sources": len(all_three),
        "models_with_cost": sorted({_model(o) for o in gens if _model(o) and (_cost(o) or 0) > 0}),
        "generations_with_cost": sum(1 for o in gens if (_cost(o) or 0) > 0),
        "scores": len(scores), "score_names": names,
    }
    p: list[str] = []
    if not obs:
        p.append("Langfuse returned no observations for the traces of this run: nothing was ingested (wrong project keys, forwarder "
                 "not forwarding, or ingestion still processing; rerun -Verify in a minute)")
    else:
        if by_src["engine"] == 0:
            p.append("no ENGINE spans (pulso.story / stage.* / generation): the engine_trace step did not reach Langfuse through the forwarder")
        if by_src["gateway"] == 0:
            p.append("no GATEWAY spans (chat <model>): they arrive as OTLP protobuf; either Langfuse did not accept application/x-protobuf "
                     "from the forwarder (see forwarder log: 'traces dropped: upstream HTTP ...'), or the gateway container was not "
                     "started with OTEL_EXPORTER_OTLP_ENDPOINT -> forwarder")
        if by_src["agent-core"] == 0:
            p.append("no AGENT-CORE spans (agentcore.*): same protobuf path as the gateway, or agent-core was not started with the OTLP "
                     "endpoint / is not the PR 48 build")
        if story_ids and len(all_three) < len(story_ids):
            p.append(f"only {len(all_three)} of {len(story_ids)} story traces carry spans from all three sources under one trace id "
                     "(missing traceparent propagation, or one exporter had not flushed yet)")
        if not gens:
            p.append("no generations at all")
        elif rep["generations_with_content"] == 0:
            p.append("no generation has input AND output: LLM_GATEWAY_TRACE_CONTENT=1 / AGENTCORE_TRACE_CONTENT=1 / engine content capture were not applied")
        if not rep["models_with_cost"]:
            p.append("no generation resolved a model with cost: the model definitions (-Models) are missing or their match pattern does not fit the reported model name")
        if rep["scores"] < expected_scores_min:
            p.append(f"{rep['scores']} scores read back, at least {expected_scores_min} expected (rubric_total and announce per story)")
    rep["problems"] = p
    return rep


def fetch_traces(client: Client, trace_ids: list[str] | None) -> tuple[dict[str, list[dict]], list[dict]]:
    ids = trace_ids if trace_ids is not None else [t["id"] for t in client.pages("/api/public/traces")]
    out: dict[str, list[dict]] = {}
    for t in ids:
        obs = list(client.pages("/api/public/observations", {"traceId": t}))
        if obs:
            out[t] = obs
    scores = [s for s in client.pages("/api/public/scores") if trace_ids is None or s.get("traceId") in set(trace_ids)]
    return out, scores


def verify(client: Client, manifest: dict, wait_secs=0, poll_secs=20, sleep=time.sleep) -> dict:
    stories, runs = list(manifest.get("stories", [])), list(manifest.get("agent_runs", []))
    ids = stories + runs
    deadline = time.monotonic() + wait_secs
    while True:
        traces, scores = fetch_traces(client, ids or None)
        rep = summarize(traces, set(stories), scores, 2 * len(stories))
        rep["expected_traces"] = len(ids)
        if len(traces) < len(ids):
            rep["problems"].insert(0, f"{len(traces)} of {len(ids)} expected traces are in Langfuse")
        if not rep["problems"] or time.monotonic() >= deadline:
            return rep
        sleep(poll_secs)


def format_report(rep: dict) -> list[str]:
    s = rep["observations_by_source"]
    lines = [
        f"traces: {rep['traces']} (expected {rep.get('expected_traces', '?')})",
        f"observations: {rep['observations']}  spans: {rep['spans']}  generations: {rep['generations']}",
        f"generations with input+output content: {rep['generations_with_content']}",
        f"observations by source: engine={s['engine']} gateway={s['gateway']} agent-core={s['agent-core']}",
        f"story traces with all three sources under one trace id: {rep['story_traces_with_all_three_sources']} of {rep['story_traces']}",
        f"models resolved with cost: {len(rep['models_with_cost'])} ({', '.join(rep['models_with_cost']) or '-'})  generations with cost: {rep['generations_with_cost']}",
        f"scores: {rep['scores']} ({', '.join(rep['score_names']) or '-'})",
    ]
    return lines


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("cmd", choices=["health", "models", "verify"])
    ap.add_argument("--env-file", action="append", default=[])
    ap.add_argument("--allow-external", action="store_true")
    ap.add_argument("--manifest")
    ap.add_argument("--wait-secs", type=int, default=0)
    ap.add_argument("--interval", type=float, default=2.1)
    a = ap.parse_args(argv)
    env = dict(os.environ)
    for f in a.env_file:
        env.update(_load_env_file(f))
    try:
        c = client_from_env(env, a.allow_external or _truthy(env.get("PULSO_O11Y_ALLOW_EXTERNAL")), min_interval=a.interval)
        if a.cmd == "health":
            st = health(c)
            print(f"langfuse health: {st}")
            return 0 if st.startswith("HTTP 200") else 1
        if a.cmd == "models":
            r = ensure_models(c)
            print(f"models: created {len(r['created'])} ({', '.join(r['created']) or '-'}), already present {len(r['existing'])}")
            return 0
        manifest = json.loads(Path(a.manifest).read_text(encoding="utf-8-sig")) if a.manifest else {}
        rep = verify(c, manifest, a.wait_secs)
        for line in format_report(rep):
            print(line)
        for p in rep["problems"]:
            print("PROBLEM: " + p)
        print("VERIFY " + ("FAILED" if rep["problems"] else "OK"))
        return 1 if rep["problems"] else 0
    except (RuntimeError, ValueError, OSError) as e:
        print("error: " + redact(str(e), []), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
