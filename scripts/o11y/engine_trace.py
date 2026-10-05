"""Engine story trace (O11Y3, plan W1-3): the ENGINE's own story as OTLP spans under the story trace id.

One story = one finding of one engine run (the value loop, `pulso run`). Spans:
  pulso.story (root; metadata finding_key, session = run id, tags stage:<final stage>, case-type:<kind>)
    stage.<sensor|scout|recompute|verifier|builder|compile|deliver|evaluate|dossier>   (duration + outcome)
      generation spans (`generation scout|verifier|builder`) per model call: model, usage, USD from the price table,
      prompt and response CONTENT when the engine recorded it (contract `pulso.model_call/1`, see docs/dev/O11Y.md)
Scores on the root trace: gate_regression_proven, rubric_total, announce, outcome_class.

The trace id is `trace_id.story_trace_id(finding_key, run_id)` with finding_key = the value-loop record `evidence_ref`.
Span ids come from `trace_id.root_span_id / stage_span_id / generation_span_id`, i.e. the SAME ids the engine puts in the
`traceparent` it sends to llm-gateway and agent-core (`traceparent_for`), so their spans hang under the stage spans.

Inputs (any combination; a recorded bundle file is enough, no engine needed):
  --story-file F      bundle {schema pulso.engine_story/1, run_id, events[], loop{}, model_calls[], verdict_story{}, dossier{}}
  --outcome F         the value-loop outcome JSON (`<work>/value-loop/<job>.json`, contract value-loop/b3-0)
  --events-file F     debug-api events (list, or {"items": [...]})
  --api URL --run-id R   read `/internal/v1/debug/runs/R/events` (and `/model-calls`) from the engine; loopback only;
                      token from env PULSO_DEBUG_TOKEN or --token-file (JSON, key --token-key); never printed
Targets (as runtrace_bridge): local (default, loopback receiver), forwarder (loopback, scores through it), langfuse
(needs --allow-external for a non-loopback host). Langfuse Cloud is never contacted by default.

  python engine_trace.py --story-file fixtures/engine_story_recorded.json --finding 0 [--target local] [--dump out.json]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import runtrace_bridge as rb  # noqa: E402  (reuse price table, attribute encoding, senders)
from trace_id import generation_span_id, root_span_id, stage_span_id, story_trace_id  # noqa: E402

SCOPE = "pulso.engine-trace"
SCHEMA_STORY = "pulso.engine_story/1"
SCHEMA_CALL = "pulso.model_call/1"
PIPELINE = ["scout", "recompute", "verifier", "builder", "compile"]  # order in reasoning::pipeline::reason
MODEL_ROLES = ["scout", "verifier", "builder"]  # order of Recording calls in Reasoned.calls
EARLY_STOPS = {"direction", "mapping", "policy"}  # stages that end the record before any model call
CONTENT_CAP = 20000


def _dt(s: str) -> datetime:
    return rb._dt(s)


def scrub_content(v):
    """Secret masking like the bridge, with a larger cap: prompts are long."""
    if isinstance(v, str):
        return rb.SECRET_RE.sub("[redacted]", v)[:CONTENT_CAP]
    if isinstance(v, dict):
        return {k: scrub_content(x) for k, x in v.items()}
    if isinstance(v, list):
        return [scrub_content(x) for x in v]
    return v


def _content(v) -> str:
    v = scrub_content(v)
    return v if isinstance(v, str) else json.dumps(v, sort_keys=True, default=str, ensure_ascii=False)


# ---------------------------------------------------------------------------------------------- story pieces

def finding_key_of(rec: dict) -> str:
    """The engine's story key for a finding: the value-loop record `evidence_ref` (stable, `ev_<16 hex>`)."""
    return str(rec.get("evidence_ref") or rec.get("finding_id") or "")


def executed_stages(rec: dict) -> list[str]:
    """Which pipeline stages ran for this record: everything up to and including the stage it ended at."""
    st = rec.get("stage")
    if st in PIPELINE:
        return PIPELINE[: PIPELINE.index(st) + 1]
    return []


def outcome_class(rec: dict) -> str:
    s = rec.get("status") or "unknown"
    if s == "proposed":
        d = (rec.get("delivery") or {}).get("status")
        return f"proposed_{d}" if d else "proposed"
    if s == "blocked":
        return f"blocked_{rec.get('reason') or 'unknown'}"
    return str(s)


def for_finding(obj: dict | None, rec: dict) -> dict | None:
    """A verdict story or dossier belongs to one finding when it names it (finding_id); unnamed ones apply to any."""
    if not obj:
        return None
    fid = obj.get("finding_id") or (obj.get("refs") or {}).get("finding_id")
    return obj if fid in (None, rec.get("finding_id")) else None


def case_type(rec: dict, story: dict) -> str:
    d = for_finding(story.get("dossier"), rec) or {}
    return str(d.get("finding_kind") or story.get("case_type") or rec.get("proposal_kind") or "unknown")


def window_of(story: dict, idx: int) -> tuple[datetime, datetime]:
    """Time window of finding `idx` from the debug events: previous finding node (or run_started) -> its own node event."""
    if story.get("window"):
        return _dt(story["window"][0]), _dt(story["window"][1])
    ev = story.get("events") or []
    started = next((e for e in ev if e.get("kind") == "run_started"), None)
    node = {e["entity_ref"]["id"]: e for e in ev if e.get("kind") == "node_status_changed"}
    this = node.get(f"finding-{idx}")
    if started is None or this is None:
        raise ValueError("no timing source: need debug events (run_started and the finding node) or a story `window`")
    prev = node.get(f"finding-{idx - 1}") if idx > 0 else None
    return _dt((prev or started)["occurred_at"]), _dt(this["occurred_at"])


def calls_for(rec: dict, story: dict) -> tuple[list[dict], bool]:
    """Model calls of this finding: recorded contract items when the engine has them, else ids reconstructed from rec.models."""
    mine = [c for c in story.get("model_calls") or []
            if c.get("evidence_ref") in (None, rec.get("evidence_ref")) and c.get("role") in MODEL_ROLES]
    if mine:
        return mine, True
    stages = [s for s in executed_stages(rec) if s in MODEL_ROLES]
    out = []
    for role, mid in zip(stages, rec.get("models") or []):
        out.append({"role": role, "model_id": mid})
    return out, False


# ---------------------------------------------------------------------------------------------- conversion

class StoryConverter:
    def __init__(self, capture_content: bool = False, environment: str = "local"):
        self.capture, self.env = capture_content, environment

    def _span(self, tid, sid, parent, name, start: datetime, end: datetime, attrs: dict, error=False, msg=None) -> dict:
        s = {"traceId": tid, "spanId": sid, "name": name, "kind": 1,
             "startTimeUnixNano": rb._ns(start), "endTimeUnixNano": rb._ns(max(end, start)),
             "attributes": rb._attrs(attrs), "status": {"code": 2 if error else 1}}
        if error and msg:
            s["status"]["message"] = str(msg)
        if parent:
            s["parentSpanId"] = parent
        return s

    def convert(self, story: dict, idx: int) -> tuple[dict, list[dict]]:
        """-> (one OTLP resourceSpans entry, trace scores) for finding `idx` of the story."""
        loop = story["loop"]
        rec = loop["findings"][idx]
        run_id = story["run_id"]
        key = story.get("finding_key") or finding_key_of(rec)
        tid = story_trace_id(key, run_id)
        root_id = root_span_id(tid)
        w0, w1 = window_of(story, idx)
        verdict, dossier = for_finding(story.get("verdict_story"), rec), for_finding(story.get("dossier"), rec)
        stages = ["sensor", *executed_stages(rec)]
        if rec.get("delivery"):
            stages.append("deliver")
        if verdict:
            stages.append("evaluate")
        if dossier:
            stages.append("dossier")
        calls, recorded = calls_for(rec, story)
        by_role: dict[str, list[dict]] = {}
        for c in calls:
            by_role.setdefault(c["role"], []).append(c)

        # durations: recorded model-call durations win; the rest of the window is split evenly (marked inferred)
        known = {s: sum(c.get("duration_ms", 0) for c in by_role.get(s, [])) / 1000.0 for s in stages
                 if s in by_role and all("duration_ms" in c for c in by_role[s])}
        total = max((w1 - w0).total_seconds(), sum(known.values()))
        unknown = [s for s in stages if s not in known]
        share = max(total - sum(known.values()), 0.0) / len(unknown) if unknown else 0.0
        t, bounds = w0, {}
        for s in stages:
            d = known.get(s, share)
            bounds[s] = (t, t + timedelta(seconds=d), s in known)
            t += timedelta(seconds=d)
        end = max(w1, t)

        cls, kind = outcome_class(rec), case_type(rec, story)
        final = rec.get("stage") or "none"
        trace_attrs = {
            "langfuse.trace.name": "pulso.story", "langfuse.environment": self.env, "langfuse.session.id": run_id,
            "langfuse.trace.tags": ["pulso", "engine", f"stage:{final}", f"case-type:{kind}"],
            "langfuse.trace.metadata.finding_key": key, "langfuse.trace.metadata.finding_id": rec.get("finding_id"),
            "langfuse.trace.metadata.metric": rec.get("metric"), "langfuse.trace.metadata.run_id": run_id,
            "langfuse.trace.metadata.outcome_class": cls, "langfuse.trace.metadata.status": rec.get("status"),
            "langfuse.trace.metadata.reason": rec.get("reason"), "langfuse.trace.metadata.final_stage": final,
            "langfuse.trace.metadata.data_source": loop.get("data_source"),
            "langfuse.trace.metadata.sensor": "claude-standin",
            "langfuse.trace.metadata.independence": (rec.get("independence") or {}).get("level"),
        }
        common = {k: v for k, v in trace_attrs.items() if k.startswith("langfuse.")}
        spans = []
        blocked = rec.get("status") == "blocked"
        for s in stages:
            b0, b1, rec_t = bounds[s]
            sid = stage_span_id(tid, s)
            a = {**common, "langfuse.observation.type": "span", "pulso.stage": s,
                 "pulso.timing": "recorded" if rec_t else "inferred"}
            err, msg = False, None
            if s == "sensor":
                a["pulso.sensor"] = "claude-standin"
                a["pulso.sensor.corroborated"] = (loop.get("summary") or {}).get("corroborated")
                a["pulso.sensor.skipped"] = (loop.get("summary") or {}).get("skipped_not_corroborated")
            elif s in PIPELINE:
                last = s == final
                a["pulso.stage.outcome"] = ("blocked" if blocked else "ok") if last else "ok"
                if last and blocked:
                    err, msg = True, rec.get("reason")
                    a["pulso.reason"] = rec.get("reason")
                if s == "compile" and rec.get("target_ref"):
                    a["pulso.target_ref"] = rec.get("target_ref")
                    a["pulso.proposal_kind"] = rec.get("proposal_kind")
            elif s == "deliver":
                d = rec["delivery"]
                a.update({"pulso.stage.outcome": d.get("status"), "pulso.reason": d.get("reason"),
                          "pulso.delivery.credential": d.get("credential")})
                if self.capture:
                    a["pulso.delivery.proposal_id"] = d.get("proposal_id")
                err, msg = d.get("status") == "denied", d.get("reason")
            elif s == "evaluate":
                a.update({"pulso.stage.outcome": verdict.get("outcome"), "pulso.reason": verdict.get("reason"),
                          "pulso.suite_id": verdict.get("suite_id"), "pulso.attempts": len(verdict.get("attempts") or []),
                          "pulso.regression_proven": verdict.get("outcome") == "regression_suite_proven"})
            elif s == "dossier":
                a.update({"pulso.stage.outcome": "announce" if dossier.get("announce") else "not_announced",
                          "pulso.reason": dossier.get("announce_reason"), "pulso.dossier.outcome": dossier.get("outcome")})
            spans.append(self._span(tid, sid, root_id, f"stage.{s}", b0, b1, a, error=err, msg=msg))
            # generation spans of this stage
            t2 = b0
            for n, c in enumerate(by_role.get(s, []), 1):
                spans.append(self._generation(tid, sid, c, n, common, t2, b1, recorded))
                t2 = t2 + timedelta(milliseconds=c.get("duration_ms", 0))

        gens = [c for cs in by_role.values() for c in cs]
        tin = sum(c.get("tokens_in") or 0 for c in gens)
        tout = sum(c.get("tokens_out") or 0 for c in gens)
        usd = sum(u for c in gens if (u := self._cost(c)) is not None)
        root_attrs = {**trace_attrs, "langfuse.observation.type": "span", "pulso.story": True,
                      "pulso.run_id": run_id, "pulso.outcome_class": cls, "pulso.final_stage": final,
                      "pulso.llm.calls": len(gens), "pulso.llm.recorded": recorded,
                      "gen_ai.usage.input_tokens": tin if recorded else None,
                      "gen_ai.usage.output_tokens": tout if recorded else None,
                      "pulso.cost_usd": round(usd, 12) if recorded else None}
        if self.capture:
            root_attrs["langfuse.trace.output"] = _content({k: rec.get(k) for k in
                                                           ("status", "reason", "stage", "rubric", "target_ref", "delivery")})
        root = self._span(tid, root_id, None, "pulso.story", w0, end, root_attrs, error=blocked, msg=rec.get("reason"))
        rs = {"resource": {"attributes": rb._attrs({"service.name": "pulso-engine", "service.namespace": "pulso",
                                                    "deployment.environment": self.env})},
              "scopeSpans": [{"scope": {"name": SCOPE, "version": "1"}, "spans": [root, *spans]}]}
        return rs, self.scores(tid, run_id, rec, verdict, dossier, cls)

    @staticmethod
    def _cost(c: dict) -> float | None:
        if c.get("tokens_in") is None and c.get("tokens_out") is None:
            return float(c["cost_usd"]) if c.get("cost_usd") is not None else None
        p = rb.price_usd(c.get("model_id"), c.get("tokens_in") or 0, c.get("tokens_out") or 0)
        return p if p is not None else (float(c["cost_usd"]) if c.get("cost_usd") is not None else None)

    def _generation(self, tid, parent, c, n, common, start, stage_end, recorded) -> dict:
        role, model = c["role"], c.get("model_id")
        dur = c.get("duration_ms")
        s0 = _dt(c["started_at"]) if c.get("started_at") else start
        e0 = s0 + timedelta(milliseconds=dur) if dur is not None else stage_end
        usd = self._cost(c)
        a = {**common, "langfuse.observation.type": "generation", "gen_ai.operation.name": "chat",
             "gen_ai.request.model": model, "gen_ai.response.model": model, "gen_ai.system": "llm-gateway",
             "pulso.role": role, "pulso.call.n": n, "pulso.call.outcome": c.get("outcome"),
             "pulso.usage.recorded": c.get("tokens_in") is not None or c.get("tokens_out") is not None}
        if a["pulso.usage.recorded"]:
            tin, tout = c.get("tokens_in") or 0, c.get("tokens_out") or 0
            a.update({"gen_ai.usage.input_tokens": tin, "gen_ai.usage.output_tokens": tout,
                      "gen_ai.usage.total_tokens": tin + tout,
                      "langfuse.observation.usage_details": json.dumps({"input": tin, "output": tout})})
        if usd is not None:
            a["gen_ai.usage.cost"] = round(usd, 12)
            a["pulso.cost.source"] = "price_table" if rb.price_usd(model, 0, 0) is not None else "reported"
        if self.capture:
            if c.get("request") is not None or c.get("prompt") is not None:
                a["langfuse.observation.input"] = _content(c.get("request") if c.get("request") is not None else c.get("prompt"))
            if c.get("response") is not None:
                a["langfuse.observation.output"] = _content(c["response"])
        bad = c.get("outcome") not in (None, "answered")
        return self._span(tid, generation_span_id(tid, role, n), parent, f"generation {role}", s0, e0, a,
                          error=bad, msg=c.get("why") or c.get("outcome"))

    def scores(self, tid, run_id, rec, verdict, dossier, cls) -> list[dict]:
        out: list[tuple[str, object, str, str]] = []
        if verdict is not None:
            out.append(("gate_regression_proven", 1 if verdict.get("outcome") == "regression_suite_proven" else 0, "BOOLEAN",
                        f"reg1 outcome {verdict.get('outcome')}: {verdict.get('reason')}"))
        if isinstance(rec.get("rubric"), dict) and isinstance(rec["rubric"].get("total"), (int, float)):
            out.append(("rubric_total", rec["rubric"]["total"], "NUMERIC", f"band {rec['rubric'].get('band')}; structural self-score"))
        if dossier is not None:
            ann, why = bool(dossier.get("announce")), dossier.get("announce_reason")
        elif verdict is not None:
            ann, why = bool(verdict.get("announce")), "verdict_story"
        else:
            ann, why = False, "not_evaluated"
        out.append(("announce", 1 if ann else 0, "BOOLEAN", f"announce_reason {why}"))
        out.append(("outcome_class", cls, "CATEGORICAL", "engine value-loop outcome"))
        return [{"id": hashlib.sha256(f"score:{run_id}:{tid}:{n}".encode()).hexdigest()[:32], "traceId": tid, "name": n,
                 "value": v, "dataType": dt, "environment": self.env, "comment": c} for n, v, dt, c in out]


# ---------------------------------------------------------------------------------------------- sources

def http_get(base: str, path: str, token: str | None) -> dict:
    rb.require_loopback(base)
    req = urllib.request.Request(base.rstrip("/") + path, headers={"Authorization": f"Bearer {token}"} if token else {})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:  # noqa: S310 (loopback enforced)
            return json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"engine HTTP {e.code} on {path.split('?')[0]}") from None
    except OSError as e:
        raise RuntimeError(f"engine unreachable: {type(e).__name__}") from None


def fetch_engine(base: str, run_id: str, token: str | None) -> dict:
    """Events of a run (paged by `after_sequence`) and the model-calls page, from the debug-api."""
    pre = f"/internal/v1/debug/runs/{urllib.parse.quote(run_id)}"
    events, after = [], 0
    while True:
        items = http_get(base, f"{pre}/events?after_sequence={after}", token).get("items") or []
        if not items:
            break
        events += items
        after = max(e["sequence"] for e in items)
    calls = http_get(base, f"{pre}/model-calls", token).get("items") or []
    return {"events": events, "model_calls": calls}


def load_story(a) -> dict:
    story: dict = {}
    if a.story_file:
        story = json.loads(Path(a.story_file).read_text(encoding="utf-8"))
    for flag, k in ((a.outcome, "loop"), (a.verdict_file, "verdict_story"), (a.dossier_file, "dossier"),
                    (a.model_calls_file, "model_calls"), (a.events_file, "events")):
        if flag:
            v = json.loads(Path(flag).read_text(encoding="utf-8"))
            story[k] = v.get("items", v) if isinstance(v, dict) and k in ("events", "model_calls") else v
    if a.run_id:
        story["run_id"] = a.run_id
    if a.finding_key:
        story["finding_key"] = a.finding_key
    if a.api:
        tok = (json.loads(Path(a.token_file).read_text(encoding="utf-8"))[a.token_key] if a.token_file
               else os.environ.get(a.token_env, "")) or None
        got = fetch_engine(a.api, story["run_id"], tok)
        story.setdefault("events", got["events"])
        if got["model_calls"]:
            story.setdefault("model_calls", got["model_calls"])
    if not story.get("loop") or not story.get("run_id"):
        raise ValueError("need a value-loop outcome (--outcome or --story-file) and a run id")
    return story


# ---------------------------------------------------------------------------------------------- main

def send_scores(target: str, api, scores: list[dict], out: Path | None) -> int:
    if out:
        with out.open("a", encoding="utf-8") as f:
            f.writelines(json.dumps(s, sort_keys=True) + "\n" for s in scores)
    if api is None:
        return 0
    for s in scores:
        api.score(s)
    return len(scores)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--story-file")
    ap.add_argument("--outcome", help="value-loop outcome JSON")
    ap.add_argument("--events-file")
    ap.add_argument("--verdict-file", help="reg1.verdict_story/1 JSON (evaluate stage, gate and announce scores)")
    ap.add_argument("--dossier-file", help="decision dossier JSON (dossier stage, announce score)")
    ap.add_argument("--model-calls-file", help="list of pulso.model_call/1 records")
    ap.add_argument("--api", help="engine base URL (loopback), e.g. http://127.0.0.1:4020")
    ap.add_argument("--token-env", default="PULSO_DEBUG_TOKEN")
    ap.add_argument("--token-file")
    ap.add_argument("--token-key", default="debug")
    ap.add_argument("--run-id")
    ap.add_argument("--finding-key", help="override the story key (default: the record evidence_ref)")
    ap.add_argument("--finding", help="finding index or evidence_ref; default: every finding", default=None)
    ap.add_argument("--target", choices=["local", "forwarder", "langfuse"], default="local")
    ap.add_argument("--allow-external", action="store_true")
    ap.add_argument("--no-content", action="store_true")
    ap.add_argument("--capture-content", action="store_true")
    ap.add_argument("--env-file", action="append", default=[])
    ap.add_argument("--environment", default="local")
    ap.add_argument("--dump", help="also write the OTLP JSON bodies (list) to this file")
    ap.add_argument("--scores-out", help="append the scores as JSONL to this file")
    ap.add_argument("--no-send", action="store_true", help="convert only (with --dump)")
    a = ap.parse_args(argv)
    env = dict(os.environ)
    for f in a.env_file:
        env.update(rb._load_env_file(f))
    allow = a.allow_external or rb._truthy(env.get("PULSO_O11Y_ALLOW_EXTERNAL"))
    capture = rb.resolve_capture(a.target, a.capture_content, a.no_content, env.get("PULSO_O11Y_CAPTURE_CONTENT"))
    secrets = [env.get(a.token_env, "")]
    try:
        story = load_story(a)
        findings = story["loop"]["findings"]
        idxs = list(range(len(findings)))
        if a.finding is not None:
            idxs = [int(a.finding)] if a.finding.isdigit() else [i for i, f in enumerate(findings) if f.get("evidence_ref") == a.finding]
            if not idxs:
                raise ValueError("finding not found")
        conv = StoryConverter(capture, a.environment)
        sender, api = (None, None) if a.no_send else rb.build_endpoints(a.target, env, allow)
        if sender is not None:
            secrets += sender.secrets
        bodies, sent_scores = [], 0
        for i in idxs:
            rs, scores = conv.convert(story, i)
            bodies.append(rs)
            if sender is not None:
                sender([rs])
                sent_scores += send_scores(a.target, api, scores, Path(a.scores_out) if a.scores_out else None)
            elif a.scores_out:
                send_scores(a.target, None, scores, Path(a.scores_out))
            tid = rs["scopeSpans"][0]["spans"][0]["traceId"]
            print(f"story finding={i} trace_id={tid} spans={len(rs['scopeSpans'][0]['spans'])} "
                  f"outcome={outcome_class(findings[i])} scores={[s['name'] for s in scores]}")
        if a.dump:
            Path(a.dump).write_text(json.dumps({"resourceSpans": bodies}, indent=1), encoding="utf-8")
        print(f"target={a.target} capture_content={capture} stories={len(bodies)} scores_sent={sent_scores}")
    except (RuntimeError, ValueError, KeyError) as e:
        print(f"error: {rb.redact(str(e), secrets)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
