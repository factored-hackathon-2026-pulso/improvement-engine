"""Run-trace bridge (O11Y1): agent-core run exports -> OTLP/HTTP JSON traces (Langfuse-compatible).

Pulls `GET /v1/export/runs`, `/v1/export/runs/{id}/events` and `/v1/export/registry-events` with a persisted cursor
(same cursor/overlap/loopback patterns as scripts/triggers/agentcore_poller.py) and turns every CLOSED run into one
trace: root span `agent.run` + one child span per audit event (node, agent step, tool call, decision, rule, handoff,
interrupt, response, guard...). Registry events become one tiny trace each.

Privacy: audit-view events carry no utterances (only keyed fingerprints). Structured reason fields are exported as
span attributes. Instance ids (call/action/decision/handoff/proposal ids, fingerprints) are never exported; the
trace id is derived from the run id. Free-form content (tool args/results, rule inputs, decision values, error text)
is exported only with PULSO_O11Y_CAPTURE_CONTENT=1 / --capture-content, after a secret scrub.

Targets (credentials from the environment only; never printed, logged, put on argv or stored in state):
  local     PULSO_O11Y_OTLP_ENDPOINT (default http://127.0.0.1:4318), loopback only, no auth.
  langfuse  LANGFUSE_BASE_URL + LANGFUSE_PUBLIC_KEY + LANGFUSE_SECRET_KEY (Basic auth); a non-loopback host needs
            https and --allow-external or PULSO_O11Y_ALLOW_EXTERNAL=1. It is the only external host ever contacted.

  python runtrace_bridge.py --once --state s.json --token-env AGENTCORE_EXPORT_TOKEN [--target langfuse]
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "triggers"))
from agentcore_poller import LOOPBACK, HttpFetch, redact, require_loopback  # noqa: E402  (reuse, do not fork)

SCOPE = "pulso.runtrace-bridge"
SEEN_CAP = 5000
# USD per token (input, output); decision events also carry their own cost_usd, used when the model is unknown
PRICES = {
    "xiaomi/mimo-v2.6-flash": (1.4e-7, 2.8e-7),
    "xiaomi/mimo-v2.6-pro": (4.35e-7, 8.7e-7),
    "z-ai/glm-5.3-flash": (1.5e-7, 5e-7),
}
EARLY_CLOSE = {"abandonment", "escalation", "revocation", "transfer"}
SECRET_RE = re.compile(r"(?i)(bearer\s+\S+|sk-[A-Za-z0-9_-]{8,}|pk-lf-\S+|eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_.-]+|"
                       r"(api[_-]?key|secret|token|password)\s*[=:]\s*\S+)")
EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)


def price_usd(model: str | None, tokens_in: int, tokens_out: int) -> float | None:
    p = PRICES.get((model or "").lower())
    return None if p is None else tokens_in * p[0] + tokens_out * p[1]


def trace_id_for(run_id: str) -> str:
    return hashlib.sha256(("trace:" + run_id).encode()).hexdigest()[:32]


def span_id_for(*parts: str) -> str:
    return hashlib.sha256(("span:" + "|".join(parts)).encode()).hexdigest()[:16]


def _dt(s: str) -> datetime:
    return datetime.fromisoformat(s.replace("Z", "+00:00")).astimezone(timezone.utc)


def _ns(d: datetime) -> str:
    delta = d - EPOCH
    return str((delta.days * 86400 + delta.seconds) * 10**9 + delta.microseconds * 1000)


def _ref(r) -> str | None:
    if isinstance(r, dict):
        return "@".join(str(r[k]) for k in ("id", "version") if r.get(k)) or None
    return None if r is None else str(r)


def _attr(k: str, v) -> dict | None:
    if v is None:
        return None
    if isinstance(v, bool):
        val = {"boolValue": v}
    elif isinstance(v, int):
        val = {"intValue": str(v)}
    elif isinstance(v, float):
        val = {"doubleValue": v}
    elif isinstance(v, (list, tuple)):
        val = {"arrayValue": {"values": [{"stringValue": str(x)} for x in v]}}
    elif isinstance(v, dict):
        val = {"stringValue": json.dumps(v, sort_keys=True, default=str)}
    else:
        val = {"stringValue": str(v)}
    return {"key": k, "value": val}


def _attrs(d: dict) -> list[dict]:
    return [a for k, v in sorted(d.items()) if (a := _attr(k, v)) is not None]


def scrub(v):
    """Content is exported only when enabled; even then secret-looking strings are masked."""
    if isinstance(v, str):
        return SECRET_RE.sub("[redacted]", v)[:2000]
    if isinstance(v, dict):
        return {k: scrub(x) for k, x in v.items()}
    if isinstance(v, list):
        return [scrub(x) for x in v]
    return v


# ---------------------------------------------------------------------------------------------- conversion

class Converter:
    def __init__(self, capture_content: bool = False, environment: str = "local"):
        self.capture, self.env = capture_content, environment

    def _content(self, v):
        return json.dumps(scrub(v), sort_keys=True, default=str)

    def _resource(self) -> dict:
        return {"attributes": _attrs({"service.name": "pulso-agent-core", "service.namespace": "pulso",
                                      "deployment.environment": self.env})}

    def run_to_trace(self, run: dict, events: list[dict]) -> dict:
        """One run + its events -> one OTLP `resourceSpans` entry (a single trace)."""
        rid = run["run_id"]
        tid = trace_id_for(rid)
        root_id = span_id_for(rid, "root")
        evs = sorted(events, key=lambda e: e.get("seq", 0))
        closed = next((e for e in reversed(evs) if e["type"] == "run_closed"), None)
        agent = run["agent"]["id"]
        closed_by = ((closed or {}).get("payload") or {}).get("closed_by")
        outcome = run.get("outcome") or ((closed or {}).get("payload") or {}).get("outcome")
        t0 = _dt(run["created_at"])
        t1 = _dt(run["closed_at"]) if run.get("closed_at") else (_dt(evs[-1]["ts"]) if evs else t0)
        early = (closed_by in EARLY_CLOSE) if closed_by else None
        common = {
            "langfuse.trace.name": f"agent.run {agent}",
            "langfuse.release": run.get("release"),
            "langfuse.version": run["agent"].get("version"),
            "langfuse.environment": self.env,
            "langfuse.session.id": rid,
            "langfuse.trace.tags": ["pulso", f"agent:{agent}", f"release:{run.get('release')}",
                                    f"locale:{run.get('locale')}"],
            "langfuse.trace.metadata.agent": agent,
            "langfuse.trace.metadata.release": run.get("release"),
            "langfuse.trace.metadata.locale": run.get("locale"),
            "langfuse.trace.metadata.outcome": outcome,
            "langfuse.trace.metadata.closed_by": closed_by,
            "langfuse.trace.metadata.closed_early": early,
            "langfuse.trace.metadata.mode": run.get("mode"),
            "langfuse.trace.metadata.principal_type": run.get("principal_type"),
        }
        spans = []
        tot = {"in": 0, "out": 0, "usd": 0.0, "calls": 0}
        for e in evs:
            span = self._event_span(e, tid, root_id, common, tot)
            if span:
                spans.append(span)
        root_attrs = {**common, "langfuse.observation.type": "agent", "gen_ai.operation.name": "invoke_agent",
                      "gen_ai.agent.name": agent, "gen_ai.agent.version": run["agent"].get("version"),
                      "pulso.agent": agent, "pulso.release": run.get("release"), "pulso.locale": run.get("locale"),
                      "pulso.outcome": outcome, "pulso.closed_by": closed_by, "pulso.closed_early": early,
                      "pulso.llm.calls": tot["calls"], "pulso.event_count": len(evs),
                      "gen_ai.usage.input_tokens": tot["in"], "gen_ai.usage.output_tokens": tot["out"],
                      "pulso.cost_usd": round(tot["usd"], 12)}
        root = self._span(tid, root_id, None, f"agent.run {agent}", t0, t1, root_attrs,
                          error=outcome not in (None, "completed", "resolved"), msg=outcome)
        return {"resource": self._resource(),
                "scopeSpans": [{"scope": {"name": SCOPE, "version": "1"}, "spans": [root, *spans]}]}

    def _span(self, tid, sid, parent, name, start: datetime, end: datetime, attrs: dict, error=False, msg=None):
        s = {"traceId": tid, "spanId": sid, "name": name, "kind": 1,
             "startTimeUnixNano": _ns(start), "endTimeUnixNano": _ns(max(end, start)),
             "attributes": _attrs(attrs), "status": {"code": 2 if error else 1}}
        if error and msg:
            s["status"]["message"] = str(msg)
        if parent:
            s["parentSpanId"] = parent
        return s

    def _event_span(self, e, tid, root_id, common, tot):
        typ, p, ts = e["type"], e.get("payload") or {}, _dt(e["ts"])
        if typ in ("run_started", "run_closed"):
            return None  # represented by the root span
        a: dict = {"langfuse.observation.type": "span", "pulso.event.type": typ, "pulso.event.seq": e.get("seq"),
                   **{k: v for k, v in common.items() if k.startswith("langfuse.")}}
        name, start, error = typ, ts, False
        lat = p.get("latency_ms")
        node = p.get("node_id")
        if node:
            a["pulso.node_id"] = node
        if lat is not None:
            start = ts - timedelta(milliseconds=lat)
        if typ == "node_entered":
            name = f"node {node}"
            a.update({"pulso.step.type": p.get("node_type"), "pulso.flow": _ref(p.get("flow")),
                      "pulso.resume_kind": p.get("resume_kind")})
        elif typ == "agent_step":
            name = f"agent_step {p.get('step')} {p.get('kind')}"
            a.update({"langfuse.observation.type": "tool" if p.get("kind") == "tool" else "span",
                      "pulso.step.type": "agent_step", "pulso.step.index": p.get("step"),
                      "pulso.step.kind": p.get("kind"), "gen_ai.tool.name": _ref(p.get("tool")),
                      "pulso.tool.status": p.get("status"), "pulso.reason.gateway_error_kind": p.get("error_kind")})
            error = p.get("kind") == "failed"
        elif typ == "tool_called":
            name = f"tool {_ref(p.get('tool'))}"
            a.update({"langfuse.observation.type": "tool", "gen_ai.operation.name": "execute_tool",
                      "gen_ai.tool.name": _ref(p.get("tool")), "pulso.tool.status": p.get("status"),
                      "pulso.tool.attempt": p.get("attempt"), "pulso.step.type": "tool_call",
                      "pulso.reason.tool_error": True if p.get("error") else None})
            error = p.get("status") not in (None, "ok", "success", "verified")
            if self.capture:
                a.update({"langfuse.observation.input": self._content(p.get("args")),
                          "langfuse.observation.output": self._content(p.get("result")),
                          "langfuse.observation.status_message": scrub(p.get("error"))})
        elif typ == "decision_made":
            model = _ref(p.get("model"))
            name = f"decision {model}"
            usd = float(p["cost_usd"]) if p.get("cost_usd") is not None else None
            tokens = p.get("tokens") or 0
            a.update({"langfuse.observation.type": "generation", "gen_ai.operation.name": "chat",
                      "gen_ai.request.model": p.get("model_version") or model,
                      "gen_ai.response.model": p.get("model_version"), "gen_ai.system": p.get("provider_used"),
                      "pulso.decision.model": model, "pulso.step.type": "decision",
                      "pulso.reason.fallback_depth": p.get("fallback_depth"),
                      "pulso.reason.fallback": (p.get("fallback_depth") or 0) > 0,
                      "pulso.reason.above_threshold": p.get("above_threshold"),
                      "pulso.reason.p_cal": p.get("p_cal"),
                      "gen_ai.usage.total_tokens": tokens, "gen_ai.usage.cost": usd,
                      "langfuse.observation.cost_details": json.dumps({"total": usd}) if usd is not None else None,
                      "langfuse.observation.usage_details": json.dumps({"total": tokens})})
            if self.capture:
                a["langfuse.observation.output"] = self._content(p.get("value"))
            tot["calls"] += 1
            tot["usd"] += usd or 0.0
        elif typ == "command_emitted":
            name = f"command {p.get('command')}"
            a.update({"langfuse.observation.type": "event", "pulso.step.type": "command",
                      "pulso.reason.command": p.get("command"), "pulso.reason.flow": p.get("flow"),
                      "pulso.reason.interrupt": p.get("interrupt"), "pulso.reason.source": p.get("source"),
                      "pulso.reason.additional_flows": p.get("additional_flows") or None,
                      "pulso.reason.above_threshold": p.get("above_threshold")})
        elif typ == "rule_evaluated":
            name = f"rule {node}"
            a.update({"langfuse.observation.type": "event", "pulso.step.type": "rule",
                      "pulso.reason.rule_result": p.get("result"), "pulso.reason.policy": _ref(p.get("policy"))})
            if self.capture:
                a["langfuse.observation.input"] = self._content(p.get("inputs"))
        elif typ in ("response_emitted", "response_failed"):
            llm = p.get("llm") or {}
            models = llm.get("models") or []
            tin, tout = llm.get("tokens_in") or 0, llm.get("tokens_out") or 0
            usd = price_usd(models[0], tin, tout) if models else None
            src = "price_table"
            if usd is None and llm.get("cost_usd") is not None:
                usd, src = float(llm["cost_usd"]), "reported"
            v = p.get("validator") or {}
            name = f"response {p.get('kind', 'failed')}"
            a.update({"langfuse.observation.type": "generation" if llm.get("calls") else "span",
                      "pulso.step.type": "response", "pulso.response.kind": p.get("kind"),
                      "gen_ai.request.model": models[0] if models else None,
                      "gen_ai.response.model": models[0] if models else None,
                      "gen_ai.usage.input_tokens": tin if llm else None,
                      "gen_ai.usage.output_tokens": tout if llm else None,
                      "gen_ai.usage.cost": usd, "pulso.cost.source": src if usd is not None else None,
                      "pulso.llm.calls": llm.get("calls"),
                      "langfuse.observation.usage_details": json.dumps({"input": tin, "output": tout}) if llm else None,
                      "langfuse.observation.cost_details": json.dumps({"total": usd}) if usd is not None else None,
                      "pulso.reason.fallback_used": p.get("fallback_used"),
                      "pulso.reason.reason_code": p.get("reason_code"),
                      "pulso.reason.validator_ok": v.get("ok"),
                      "pulso.reason.validator_failures": v.get("failures") or None,
                      "pulso.reason.regenerations": v.get("regenerations")})
            if llm.get("latency_ms"):
                start = ts - timedelta(milliseconds=llm["latency_ms"])
            if llm:
                tot["in"] += tin
                tot["out"] += tout
                tot["calls"] += llm.get("calls") or 0
                tot["usd"] += usd or 0.0
            error = typ == "response_failed"
        elif typ == "turn_started":
            g = p.get("guards") or {}
            a.update({"pulso.step.type": "turn", "pulso.reason.lang_decision": (g.get("lang") or {}).get("decision"),
                      "pulso.reason.injection_flagged": (g.get("injection") or {}).get("flagged"),
                      "pulso.reason.size_ok": g.get("size_ok")})
        elif typ == "turn_completed":
            name = "turn"
            if p.get("duration_ms") is not None:
                start = ts - timedelta(milliseconds=p["duration_ms"])
            a.update({"pulso.step.type": "turn", "pulso.turn.entry": p.get("entry"),
                      "pulso.turn.degraded": p.get("degraded"), "pulso.turn.awaiting": p.get("awaiting"),
                      **{f"pulso.turn.stage.{k}": v for k, v in (p.get("stages") or {}).items()}})
        elif typ == "knowledge_read":
            a.update({"langfuse.observation.type": "retriever", "pulso.step.type": "knowledge_read",
                      "pulso.knowledge.purpose": p.get("purpose"), "pulso.reason.result": p.get("result"),
                      "pulso.reason.reason": p.get("reason"), "pulso.knowledge.refs": len(p.get("refs") or []),
                      "pulso.reason.filtered_out": [f.get("reason") for f in p.get("filtered_out") or []] or None})
        elif typ == "escalated":
            name = "handoff escalated"
            a.update({"langfuse.observation.type": "event", "pulso.step.type": "handoff",
                      "pulso.reason.reason_code": p.get("reason_code"), "pulso.handoff.queue": p.get("target_queue"),
                      "pulso.handoff.priority": p.get("priority")})
        elif typ == "handoff_resolved":
            name = "handoff resolved"
            a.update({"langfuse.observation.type": "event", "pulso.step.type": "handoff",
                      "pulso.handoff.resolution_code": p.get("resolution_code"),
                      "pulso.handoff.quality": p.get("handoff_quality")})
        elif typ in ("run_transferred", "transfer_received", "transfer_rejected"):
            name = typ.replace("_", " ")
            a.update({"langfuse.observation.type": "event", "pulso.step.type": "handoff",
                      "pulso.reason.reason": p.get("reason") or p.get("reason_code"),
                      "pulso.transfer.to_agent": _ref(p.get("to_agent"))})
        elif typ in ("step_up_requested", "expiry_evaluated", "injection_flagged", "access_denied",
                     "action_confirmed", "action_cancelled", "action_dispatched", "action_verified"):
            a["langfuse.observation.type"] = ("guardrail" if typ in ("injection_flagged", "access_denied")
                                              else "tool" if typ.startswith("action_") else "event")
            a["pulso.step.type"] = "interrupt" if typ in ("step_up_requested", "expiry_evaluated") else typ
            a.update({"pulso.reason.required_level": p.get("required_level"), "pulso.reason.attempt": p.get("attempt"),
                      "pulso.reason.expired": p.get("expired"), "pulso.reason.signals": p.get("signals"),
                      "pulso.reason.ruleset": p.get("ruleset"), "pulso.reason.scope": p.get("scope"),
                      "pulso.reason.reason": p.get("reason"), "pulso.reason.source": p.get("source"),
                      "pulso.reason.result": p.get("result"), "gen_ai.tool.name": _ref(p.get("tool"))})
        else:
            a["langfuse.observation.type"] = "event"
        for k in ("pulso.reason.above_threshold", "pulso.reason.p_cal"):
            if isinstance(a.get(k), dict):
                a[k] = json.dumps(a[k], sort_keys=True)
        return self._span(tid, span_id_for(e["run_id"], e["event_id"]), root_id, name, start, ts, a, error=error)

    def scores_for(self, run: dict, resource_spans: dict) -> list[dict]:
        """Trace scores (sent after the trace): completion gate, early close. Deterministic ids -> idempotent."""
        rid, tid = run["run_id"], trace_id_for(run["run_id"])
        root = next(sp for sp in resource_spans["scopeSpans"][0]["spans"] if "parentSpanId" not in sp)
        at = {a["key"]: a["value"] for a in root["attributes"]}
        out = []
        if "pulso.outcome" in at:
            out.append(("run_completed", 1 if at["pulso.outcome"]["stringValue"] == "completed" else 0))
        if "pulso.closed_early" in at:
            out.append(("closed_early", 1 if at["pulso.closed_early"]["boolValue"] else 0))
        return [{"id": hashlib.sha256(f"score:{rid}:{n}".encode()).hexdigest()[:32], "traceId": tid, "name": n,
                 "value": v, "dataType": "BOOLEAN", "environment": self.env,
                 "comment": "derived from agent-core run export"} for n, v in out]

    def registry_event_trace(self, pos: int, e: dict) -> dict:
        key = f"registry:{pos}:{e['type']}:{e.get('release_id')}"
        t = _dt(e["at"])
        a = {"langfuse.observation.type": "event", "langfuse.trace.name": f"registry.{e['type']}",
             "langfuse.environment": self.env, "langfuse.release": e.get("release_id"),
             "langfuse.trace.tags": ["pulso", "registry"], "pulso.event.type": e["type"],
             "pulso.registry.actor": e.get("actor"), "pulso.registry.origin": e.get("origin"),
             "pulso.registry.principal_type": e.get("principal_type"), "pulso.release": e.get("release_id"),
             "langfuse.trace.metadata.event_type": e["type"], "langfuse.trace.metadata.origin": e.get("origin")}
        sp = self._span(trace_id_for(key), span_id_for(key), None, f"registry.{e['type']}", t, t, a)
        return {"resource": self._resource(),
                "scopeSpans": [{"scope": {"name": SCOPE, "version": "1"}, "spans": [sp]}]}


# ---------------------------------------------------------------------------------------------- transport

class OtlpSender:
    """POST `{"resourceSpans": [...]}` to the traces URL. The auth header is held in memory only."""

    def __init__(self, url: str, headers: dict | None = None, secrets: list[str] | None = None):
        self.url, self.headers, self.secrets = url, headers or {}, secrets or []

    def __call__(self, resource_spans: list[dict]) -> None:
        body = json.dumps({"resourceSpans": resource_spans}, separators=(",", ":")).encode()
        req = urllib.request.Request(self.url, data=body, method="POST",
                                     headers={"Content-Type": "application/json", **self.headers})
        try:
            urllib.request.urlopen(req, timeout=30).close()  # noqa: S310 (host policy enforced by build_sender)
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"OTLP receiver HTTP {e.code}") from None
        except OSError as e:
            raise RuntimeError(redact(f"OTLP receiver unreachable: {type(e).__name__}", self.secrets)) from None


class LangfuseApi:
    """Public REST API (scores, models): same host policy, in-memory Basic auth, throttled for Hobby (30 req/min)."""

    def __init__(self, base: str, headers: dict, secrets: list[str], min_interval: float = 2.1, sleep=time.sleep):
        self.base, self.headers, self.secrets, self.min_interval, self.sleep = base, headers, secrets, min_interval, sleep
        self._last = 0.0

    def request(self, method: str, path: str, body: dict | None = None) -> dict:
        wait = self.min_interval - (time.monotonic() - self._last)
        if wait > 0:
            self.sleep(wait)
        self._last = time.monotonic()
        req = urllib.request.Request(self.base + path, method=method,
                                     data=None if body is None else json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json", **self.headers})
        try:
            with urllib.request.urlopen(req, timeout=30) as r:  # noqa: S310 (host policy enforced by build_endpoints)
                raw = r.read()
                return json.loads(raw) if raw else {}
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"Langfuse API HTTP {e.code} on {method} {path.split('?')[0]}") from None
        except OSError as e:
            raise RuntimeError(redact(f"Langfuse API unreachable: {type(e).__name__}", self.secrets)) from None

    def score(self, sc: dict) -> None:
        self.request("POST", "/api/public/scores", sc)


def register_models(api: LangfuseApi) -> dict:
    """Idempotent: one model definition per PRICES entry unless one with that name already exists."""
    have = {m["modelName"] for m in api.request("GET", "/api/public/models?limit=100").get("data", [])}
    made = []
    for name, (pin, pout) in PRICES.items():
        if name in have:
            continue
        api.request("POST", "/api/public/models", {
            "modelName": name, "matchPattern": "(?i)^" + re.escape(name) + "$", "unit": "TOKENS",
            "inputPrice": pin, "outputPrice": pout})
        made.append(name)
    return {"created": made, "existing": sorted(have & set(PRICES))}


def _truthy(v: str | None) -> bool:
    return (v or "").strip().lower() in ("1", "true", "yes", "on")


def build_sender(target: str, env: dict, allow_external: bool) -> OtlpSender:
    return build_endpoints(target, env, allow_external)[0]


def build_endpoints(target: str, env: dict, allow_external: bool) -> tuple[OtlpSender, LangfuseApi | None]:
    if target == "local":
        base = env.get("PULSO_O11Y_OTLP_ENDPOINT") or "http://127.0.0.1:4318"
        require_loopback(base)
        return OtlpSender(base.rstrip("/") + "/v1/traces"), None
    if target != "langfuse":
        raise ValueError("unknown target")
    base, pk, sk = env.get("LANGFUSE_BASE_URL", ""), env.get("LANGFUSE_PUBLIC_KEY", ""), env.get("LANGFUSE_SECRET_KEY", "")
    if not (base and pk and sk):
        raise ValueError("langfuse target needs LANGFUSE_BASE_URL, LANGFUSE_PUBLIC_KEY and LANGFUSE_SECRET_KEY "
                         "in the environment")
    u = urllib.parse.urlparse(base)
    if not u.hostname or u.scheme not in ("http", "https"):
        raise ValueError("invalid LANGFUSE_BASE_URL")
    if u.hostname not in LOOPBACK:
        if not allow_external:
            raise ValueError("external export needs --allow-external or PULSO_O11Y_ALLOW_EXTERNAL=1")
        if u.scheme != "https":
            raise ValueError("external export requires https")
    auth = base64.b64encode(f"{pk}:{sk}".encode()).decode()
    secrets = [pk, sk, auth]
    root = f"{u.scheme}://{u.netloc}"
    return (OtlpSender(root + "/api/public/otel/v1/traces",
                       {"Authorization": f"Basic {auth}", "x-langfuse-ingestion-version": "4"}, secrets),
            LangfuseApi(root, {"Authorization": f"Basic {auth}"}, secrets))


# ---------------------------------------------------------------------------------------------- bridge

class Bridge:
    def __init__(self, *, fetch: Callable[[str, dict], dict], send: Callable[[list[dict]], None], state_path: Path,
                 converter: Converter | None = None, page_limit: int = 100, overlap: int = 0, batch: int = 20,
                 only_agent: str | None = None, score: Callable[[dict], None] | None = None):
        self.score = score
        self.fetch, self.send, self.state_path = fetch, send, Path(state_path)
        self.conv = converter or Converter()
        self.page_limit, self.overlap, self.batch, self.only_agent = page_limit, overlap, batch, only_agent
        self.state = {"runs_cursor": 0, "registry_after": 0, "exported": [], "score_queue": []}
        if self.state_path.exists():
            self.state.update(json.loads(self.state_path.read_text(encoding="utf-8")))
        self._seen = set(self.state["exported"])

    def _save(self) -> None:
        self.state["exported"] = self.state["exported"][-SEEN_CAP:]
        tmp = self.state_path.with_suffix(".tmp")
        tmp.write_text(json.dumps(self.state, sort_keys=True), encoding="utf-8")
        os.replace(tmp, self.state_path)

    def _pages(self, path: str, after: int):
        while True:
            page = self.fetch(path, {"after": after, "limit": self.page_limit})
            yield page["items"]
            nxt = page["next_after"]
            if nxt == after or not page["items"]:
                return
            after = nxt

    def _flush(self, pending: list[tuple[str, dict]]) -> int:
        n = 0
        for i in range(0, len(pending), self.batch):
            chunk = pending[i:i + self.batch]
            self.send([rs for _, rs in chunk])
            for key, _ in chunk:
                self._seen.add(key)
                self.state["exported"].append(key)
            n += len(chunk)
        return n

    def _events(self, rid: str) -> list[dict]:
        return [e for evs in self._pages(f"/v1/export/runs/{rid}/events", -1) for e in evs]

    def poll_once(self) -> dict:
        sent_runs = sent_reg = spans = 0
        try:
            start = self.state["runs_cursor"]
            last, hold, pending = start, None, []
            for items in self._pages("/v1/export/runs", max(0, start - self.overlap)):
                for r in items:
                    if r.get("closed_at") is None:
                        hold = r["cursor"] - 1 if hold is None else hold  # keep open runs inside the next window
                        continue
                    if hold is None:
                        last = max(last, r["cursor"])
                    key = "run:" + r["run_id"]
                    if key in self._seen or (self.only_agent and r["agent"]["id"] != self.only_agent):
                        continue
                    rs = self.conv.run_to_trace(r, self._events(r["run_id"]))
                    spans += len(rs["scopeSpans"][0]["spans"])
                    pending.append((key, rs))
                    self.state["score_queue"].extend(self.conv.scores_for(r, rs))
            sent_runs = self._flush(pending)
            self.state["runs_cursor"] = last
            pos, last_r, pending = max(0, self.state["registry_after"] - self.overlap), self.state["registry_after"], []
            for items in self._pages("/v1/export/registry-events", pos):
                for e in items:
                    pos += 1
                    last_r = max(last_r, pos)
                    key = f"reg:{pos}:{e['type']}:{e.get('release_id')}"
                    if key not in self._seen:
                        pending.append((key, self.conv.registry_event_trace(pos, e)))
            sent_reg = self._flush(pending)
            self.state["registry_after"] = last_r
            scored = self._drain_scores()
        finally:
            self._save()
        return {"run_traces": sent_runs, "registry_traces": sent_reg, "run_spans": spans, "scores": scored}

    def _drain_scores(self) -> int:
        n = 0
        if self.score is None:
            return n
        while self.state["score_queue"]:
            self.score(self.state["score_queue"][0])
            self.state["score_queue"].pop(0)
            n += 1
        return n


def _load_env_file(path: str) -> dict:
    out = {}
    for line in Path(path).read_text(encoding="utf-8-sig").splitlines():
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            k, v = line.split("=", 1)
            v = v.strip()
            if len(v) >= 2 and v[0] == v[-1] and v[0] in "'\"":
                v = v[1:-1]
            out[k.strip()] = v
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--core-url", default="http://127.0.0.1:8001")
    ap.add_argument("--target", choices=["local", "langfuse"], default="local")
    ap.add_argument("--allow-external", action="store_true")
    ap.add_argument("--capture-content", action="store_true")
    ap.add_argument("--state", default="o11y-state.json")
    ap.add_argument("--token-env", default="AGENTCORE_EXPORT_TOKEN")
    ap.add_argument("--token-file", help="JSON file holding the token (local dev stack); never printed")
    ap.add_argument("--token-key", default="admin")
    ap.add_argument("--env-file", action="append", default=[], help="load KEY=VALUE into this process only")
    ap.add_argument("--only-agent")
    ap.add_argument("--overlap", type=int, default=0)
    ap.add_argument("--environment", default="local")
    ap.add_argument("--no-scores", action="store_true")
    ap.add_argument("--register-models", action="store_true",
                    help="create the price-table model definitions in Langfuse and exit")
    ap.add_argument("--once", action="store_true")
    ap.add_argument("--interval-secs", type=int, default=30)
    a = ap.parse_args(argv)
    env = dict(os.environ)
    for f in a.env_file:
        env.update(_load_env_file(f))
    tok = (json.loads(Path(a.token_file).read_text(encoding="utf-8"))[a.token_key]
           if a.token_file else env.get(a.token_env, ""))
    allow = a.allow_external or _truthy(env.get("PULSO_O11Y_ALLOW_EXTERNAL"))
    capture = a.capture_content or _truthy(env.get("PULSO_O11Y_CAPTURE_CONTENT"))
    secrets = [tok]
    try:
        sender, api = build_endpoints(a.target, env, allow)
        secrets += sender.secrets
        if a.register_models:
            if api is None:
                raise ValueError("--register-models needs --target langfuse")
            print("models", json.dumps(register_models(api)))
            return 0
        b = Bridge(fetch=HttpFetch(a.core_url, tok), send=sender,
                   score=None if (api is None or a.no_scores) else api.score, state_path=Path(a.state), overlap=a.overlap,
                   converter=Converter(capture, a.environment), only_agent=a.only_agent)
        while True:
            r = b.poll_once()
            print(f"target={a.target} capture_content={capture} " + " ".join(f"{k}={v}" for k, v in r.items())
                  + f" runs_cursor={b.state['runs_cursor']} registry_after={b.state['registry_after']}")
            if a.once:
                break
            time.sleep(a.interval_secs)
    except (RuntimeError, ValueError) as e:
        print(f"error: {redact(str(e), secrets)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
