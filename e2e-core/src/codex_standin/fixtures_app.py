"""control-api + lab-broker + scripted LLM doubles (annex D.3, A03, plan 17.3.8), ONE process.

DOUBLES, always declared: this is not Codex's control-api or broker. It enforces the agreed transport rules the
runtime depends on: A03 service JWTs (kid bound to (iss, aud), singular scope, tenant claim, receiver-owned jti
replay), binding callback exactly-once per (tenant, command_key) with digest/job conflicts, tenant-scoped artifacts,
lab/wiki/sandbox routes (revisioned, idempotent actions), binding-scoped authorisation checks, and an OpenAI-
compatible scripted chat endpoint (no real LLM). `/_e2e/*` is the driver's admin channel (state, knobs, scripts)."""

from __future__ import annotations

import hashlib
import json
import threading
import time
from collections import defaultdict
from datetime import UTC, datetime, timedelta
from typing import Any

import rfc8785
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse

from codex_standin.jwtsvc import Denied, KeyRing, Verifier

BROKER = "/internal/v1/broker"
HEX = hashlib.sha256


def tag(tenant: str) -> str:
    return HEX(tenant.encode()).hexdigest()[:6]


def lab_refs(tenant: str) -> dict[str, str]:
    """Deterministic, tenant-scoped lab references: the driver uses the same function to script the model."""
    t = tag(tenant)
    return {"session": f"sess-{t}", "result": f"res-{t}", "receipt": f"rcpt-{t}",
            "digest": HEX(f"res:{tenant}".encode()).hexdigest()}


class World:
    def __init__(self, ring: KeyRing, now: Any = time.time) -> None:
        self.lock = threading.RLock()
        self.control = Verifier(ring, now)  # one verifier (one jti store) per receiver
        self.broker = Verifier(ring, now)
        self.bindings: dict[tuple[str, str], dict[str, Any]] = {}  # (tenant, command_key)
        self.jobs: dict[tuple[str, str], str] = {}  # (tenant, job_id) -> command_key
        self.binding_effects: dict[tuple[str, str], int] = defaultdict(int)  # (tenant, job) -> effects
        self.binding_refs: dict[str, str] = {}  # task_binding_ref -> tenant
        self.deny_operations: set[str] = set()
        self.artifacts: dict[tuple[str, str], dict[str, Any]] = {}
        self.wiki: dict[tuple[str, str], str] = {}
        self.sessions: dict[str, dict[str, Any]] = {}  # bank sessions
        self.bank_effects: dict[tuple[str, str], int] = defaultdict(int)  # (tenant, action_key) -> applied effects
        self.requests: list[dict[str, Any]] = []
        self.faults: dict[str, list[str]] = defaultdict(list)  # route class -> queued fault modes
        self.cross_tenant_denials = 0
        self.llm_rules: list[dict[str, Any]] = []
        self.llm_calls: list[dict[str, Any]] = []
        self.llm_unscripted = 0

    def put_artifact(self, tenant: str, art_id: str, content: Any,
                     media_type: str = "application/json") -> dict[str, Any]:
        digest = HEX(rfc8785.dumps(content)).hexdigest()
        body = {"schema_version": "1", "artifact": {"id": art_id, "digest": "sha256:" + digest,
                                                    "media_type": media_type},
                "encoding": "json", "content": content, "byte_length": len(rfc8785.dumps(content))}
        with self.lock:
            self.artifacts[(tenant, art_id)] = body
        return body["artifact"]

    def fault(self, route: str) -> str | None:
        with self.lock:
            q = self.faults.get(route)
            return q.pop(0) if q else None

    def log(self, route: str, tenant: str | None, request: Request, body: Any, status: int) -> None:
        with self.lock:
            self.requests.append({"route": route, "tenant": tenant, "method": request.method,
                                  "path": request.url.path, "status": status, "body": body, "at": time.time()})

    def snapshot(self) -> dict[str, Any]:
        with self.lock:
            return {
                "bindings": [{"tenant": k[0], "command_key": k[1], **b} for k, b in self.bindings.items()],
                "binding_effects": {f"{k[0]}|{k[1]}": v for k, v in self.binding_effects.items()},
                "bank_effects": {f"{k[0]}|{k[1]}": v for k, v in self.bank_effects.items()},
                "requests": list(self.requests), "cross_tenant_denials": self.cross_tenant_denials,
                "llm_calls": list(self.llm_calls), "llm_unscripted": self.llm_unscripted,
                "accepted_tokens": {"control": list(self.control.accepted), "broker": list(self.broker.accepted)},
                "sessions": {k: {"tenant": s["tenant"], "revision": s["revision"], "closed": s["closed"],
                                 "actions": len(s["actions"])} for k, s in self.sessions.items()},
                "binding_refs": dict(self.binding_refs), "deny_operations": sorted(self.deny_operations)}

    def llm_answer(self, system: str, user: str) -> dict[str, Any] | None:
        with self.lock:
            for rule in self.llm_rules:
                m = rule["match"]
                if m.get("system_contains", "") not in system or m.get("user_contains", "") not in user:
                    continue
                if not rule["responses"]:
                    continue
                if len(rule["responses"]) == 1 and rule.get("repeat_last"):
                    out = rule["responses"][0]
                else:
                    out = rule["responses"].pop(0)
                self.llm_calls.append({"rule": rule["id"], "system_len": len(system), "user_len": len(user)})
                return out  # type: ignore[no-any-return]
            self.llm_unscripted += 1
            self.llm_calls.append({"rule": None, "system_len": len(system), "user_len": len(user),
                                   "user_head": user[:200]})
            return None


def _err(status: int, code: str, **extra: Any) -> JSONResponse:
    return JSONResponse({"code": code, **extra}, status_code=status)


def create_app(world: World, ingest: FastAPI | None = None) -> FastAPI:
    app = FastAPI(docs_url=None, redoc_url=None, openapi_url=None)
    app.state.world = world

    def authn(ver: Verifier, request: Request, aud: str, scope: str, purpose: str | None = None) -> dict[str, Any]:
        header = request.headers.get("authorization", "")
        scheme, _, token = header.partition(" ")
        if scheme.lower() != "bearer" or not token:
            raise Denied("missing_bearer")
        return ver.verify(token, aud=aud, scope=scope, purpose=purpose)

    # ---- control-api: binding callback (CAP-27) -------------------------------------------------------
    @app.post("/internal/v1/core-task-bindings")
    async def binding(request: Request) -> JSONResponse:
        body = json.loads(await request.body() or b"{}")
        try:
            claims = authn(world.control, request, "control-api", "binding", "core_task_binding")
        except Denied as exc:
            world.log("auth_denied", None, request, {"reason": exc.reason}, exc.status)
            return _err(exc.status, "pulso:auth_" + exc.reason)
        tenant = claims["tenant_id"]
        if body.get("tenant_id") != tenant:
            world.log("binding", tenant, request, None, 403)
            return _err(403, "tenant_mismatch")
        key = request.headers.get("idempotency-key", "")
        if key != body.get("command_key"):
            return _err(422, "idempotency_key_mismatch")
        mode = world.fault("bind")
        with world.lock:
            prior = world.bindings.get((tenant, key))
            if prior is not None and prior["request_digest"] != body["request_digest"]:
                world.log("binding", tenant, request, None, 409)
                return _err(409, "digest_mismatch")
            owner = world.jobs.get((tenant, body["job_id"]))
            if prior is None and owner not in (None, key):
                world.log("binding", tenant, request, None, 409)
                return _err(409, "binding_conflict")
            if mode == "503":  # before any effect
                world.log("binding", tenant, request, None, 503)
                return _err(503, "unavailable")
            if prior is None:  # the single effect of this binding
                world.bindings[(tenant, key)] = {"request_digest": body["request_digest"], "job_id": body["job_id"],
                                                 "core_run_id": body["core_run_id"], "attempt": body["attempt"],
                                                 "task_binding_ref": body["task_binding_ref"]}
                world.jobs[(tenant, body["job_id"])] = key
                world.binding_effects[(tenant, body["job_id"])] += 1
                world.binding_refs[body["task_binding_ref"]] = tenant
            if mode == "applied_then_503":  # effect committed, answer lost: the runtime must NOT claim confirmed
                world.log("binding", tenant, request, None, 503)
                return _err(503, "unavailable")
            world.log("binding", tenant, request, None, 200)
            return JSONResponse({"schema_version": "1", "state": "confirmed", "tenant_id": tenant,
                                 "job_id": body["job_id"]})

    # ---- lab-broker (D.3) -------------------------------------------------------------------------------
    def broker_gate(request: Request, scope: str) -> dict[str, Any] | JSONResponse:
        try:
            return authn(world.broker, request, "lab-broker", scope)
        except Denied as exc:
            world.log("auth_denied", None, request, {"reason": exc.reason, "scope": scope}, exc.status)
            return _err(exc.status, "pulso:auth_" + exc.reason)

    @app.post(BROKER + "/authorizations/check")
    async def authz(request: Request) -> JSONResponse:
        claims = broker_gate(request, "authorization_check")
        if isinstance(claims, JSONResponse):
            return claims
        body = json.loads(await request.body() or b"{}")
        tenant = claims["tenant_id"]
        allowed, reason = True, None
        if world.binding_refs.get(body.get("binding_ref")) != tenant:
            allowed, reason = False, "binding_unknown"  # unknown or another tenant's binding: never allowed
        elif body.get("operation") in world.deny_operations:
            allowed, reason = False, "revoked"
        world.log("authz", tenant, request, {"operation": body.get("operation"),
                                              "binding_ref": body.get("binding_ref"),
                                              "payload_digest": body.get("payload_digest"), "allowed": allowed}, 200)
        until = (datetime.now(UTC) + timedelta(seconds=60)).strftime("%Y-%m-%dT%H:%M:%SZ")
        return JSONResponse({"allowed": allowed, "authorization_revision": 1, "valid_until": until,
                             "reason_code": reason})

    @app.get(BROKER + "/artifacts/{art_id}")
    def artifact(art_id: str, request: Request) -> JSONResponse:
        claims = broker_gate(request, "artifact_read")
        if isinstance(claims, JSONResponse):
            return claims
        tenant = claims["tenant_id"]
        body = world.artifacts.get((tenant, art_id))
        if body is None:
            if any(k[1] == art_id for k in world.artifacts):
                with world.lock:
                    world.cross_tenant_denials += 1
            world.log("artifact", tenant, request, {"id": art_id}, 404)
            return _err(404, "artifact_not_found")
        world.log("artifact", tenant, request, {"id": art_id}, 200)
        return JSONResponse(body)

    @app.post(BROKER + "/lab/sessions")
    async def lab_open(request: Request) -> JSONResponse:
        claims = broker_gate(request, "lab")
        if isinstance(claims, JSONResponse):
            return claims
        refs = lab_refs(claims["tenant_id"])
        world.log("lab_open", claims["tenant_id"], request, None, 200)
        return JSONResponse({"session_ref": refs["session"], "revision": 3, "manifest_digest": "d" * 64,
                             "limits": {"max_rows": 200}, "table_catalog": []})

    @app.post(BROKER + "/lab/sessions/{sid}/queries")
    async def lab_query(sid: str, request: Request) -> JSONResponse:
        claims = broker_gate(request, "lab")
        if isinstance(claims, JSONResponse):
            return claims
        body = json.loads(await request.body() or b"{}")
        tenant = claims["tenant_id"]
        if sid != lab_refs(tenant)["session"]:
            with world.lock:
                world.cross_tenant_denials += 1
            return _err(404, "session_not_found")
        world.log("lab_query", tenant, request, {"sql": body.get("sql")}, 202)
        ref = "q-" + str(body["query_key"])[:8]
        return JSONResponse({"query_ref": ref, "status_url": "/lab/queries/" + ref}, status_code=202)

    @app.get(BROKER + "/lab/queries/{qid}")
    def lab_state(qid: str, request: Request) -> JSONResponse:
        claims = broker_gate(request, "lab")
        if isinstance(claims, JSONResponse):
            return claims
        refs = lab_refs(claims["tenant_id"])
        return JSONResponse({"state": "completed", "receipt_ref": refs["receipt"], "result_ref": refs["result"],
                             "reason_code": None})

    @app.get(BROKER + "/lab/results/{rid}")
    def lab_result(rid: str, request: Request) -> JSONResponse:
        claims = broker_gate(request, "lab")
        if isinstance(claims, JSONResponse):
            return claims
        tenant = claims["tenant_id"]
        refs = lab_refs(tenant)
        if rid != refs["result"]:
            with world.lock:
                world.cross_tenant_denials += 1
            world.log("lab_result", tenant, request, {"id": rid}, 404)
            return _err(404, "result_not_found")
        world.log("lab_result", tenant, request, {"id": rid}, 200)
        return JSONResponse({"columns": [{"name": "event_count", "type": "int", "data_class": "public"},
                                         {"name": "free_text", "type": "text", "data_class": "untrusted_text"}],
                             "rows": [[1, f"row-of-{tenant}"], [2, "x"]], "truncated": False, "next_cursor": None,
                             "total_rows": 2, "receipt_ref": refs["receipt"], "result_digest": refs["digest"]})

    @app.get(BROKER + "/lab/sessions/{sid}")
    def lab_session(sid: str, request: Request) -> JSONResponse:
        claims = broker_gate(request, "lab")
        if isinstance(claims, JSONResponse):
            return claims
        return JSONResponse({"state": "open", "revision": 3, "current_query_ref": None, "limits": {}})

    @app.post(BROKER + "/lab/sessions/{sid}/close")
    async def lab_close(sid: str, request: Request) -> JSONResponse:
        claims = broker_gate(request, "lab")
        if isinstance(claims, JSONResponse):
            return claims
        return JSONResponse({"state": "closed"})

    @app.post(BROKER + "/wiki/read")
    async def wiki_read(request: Request) -> JSONResponse:
        claims = broker_gate(request, "wiki")
        if isinstance(claims, JSONResponse):
            return claims
        body = json.loads(await request.body() or b"{}")
        tenant = claims["tenant_id"]
        world.log("wiki_read", tenant, request, {"paths": body.get("paths")}, 200)
        entries = []
        for p in body.get("paths", []):
            text = world.wiki.get((tenant, p), f"[{tenant}] page {p}")
            entries.append({"path": p, "content": text, "digest": HEX(text.encode()).hexdigest(), "evidence_refs": []})
        return JSONResponse({"entries": entries, "base_digest": HEX(b"base:" + tenant.encode()).hexdigest()})

    @app.post(BROKER + "/wiki/explore")
    async def wiki_explore(request: Request) -> JSONResponse:
        claims = broker_gate(request, "wiki")
        if isinstance(claims, JSONResponse):
            return claims
        return JSONResponse({"entries": [], "next_cursor": None, "truncated": False})

    @app.post(BROKER + "/wiki/transform")
    async def wiki_transform(request: Request) -> JSONResponse:
        claims = broker_gate(request, "wiki")
        if isinstance(claims, JSONResponse):
            return claims
        return JSONResponse({"scratch_ref": "scr-1", "diff_ref": "diff-1", "manifest_digest": "c" * 64,
                             "violations": []})

    # ---- sandbox bank (D.4): revisioned sessions, idempotent actions, tenant-scoped -------------------------
    sb = BROKER + "/sandbox"

    def own(claims: dict[str, Any], sid: str) -> dict[str, Any] | None:
        s = world.sessions.get(sid)
        if s is None:
            return None
        if s["tenant"] != claims["tenant_id"]:
            with world.lock:
                world.cross_tenant_denials += 1
            return None
        return s

    @app.post(sb + "/sessions")
    async def sb_open(request: Request) -> JSONResponse:
        claims = broker_gate(request, "sandbox")
        if isinstance(claims, JSONResponse):
            return claims
        body = json.loads(await request.body() or b"{}")
        if world.fault("sandbox_open") == "503":
            return _err(503, "unavailable")
        with world.lock:
            sid = f"bank-{tag(claims['tenant_id'])}-{len(world.sessions) + 1}"
            world.sessions[sid] = {"tenant": claims["tenant_id"], "revision": 0, "actions": {}, "closed": None,
                                   "open": body}
        world.log("sandbox_open", claims["tenant_id"], request, body, 200)
        return JSONResponse({"session_ref": sid, "revision": 0, "initial_state_digest": "sha256:" + "1" * 64})

    @app.post(sb + "/sessions/{sid}/reset")
    async def sb_reset(sid: str, request: Request) -> JSONResponse:
        claims = broker_gate(request, "sandbox")
        if isinstance(claims, JSONResponse):
            return claims
        s = own(claims, sid)
        if s is None:
            return _err(404, "session_not_found")
        s["revision"] = 0
        s["actions"].clear()
        return JSONResponse({"session_ref": sid, "revision": 0, "initial_state_digest": "sha256:" + "1" * 64})

    @app.post(sb + "/sessions/{sid}/actions")
    async def sb_act(sid: str, request: Request) -> JSONResponse:
        claims = broker_gate(request, "sandbox")
        if isinstance(claims, JSONResponse):
            return claims
        s = own(claims, sid)
        if s is None:
            return _err(404, "session_not_found")
        body = json.loads(await request.body() or b"{}")
        key, tenant = body["action_key"], claims["tenant_id"]
        mode = world.fault("sandbox_act")
        with world.lock:
            prior = s["actions"].get(key)
            if prior is not None:
                if prior["payload"] != body["action"]:
                    return _err(409, "action_key_conflict")
                return JSONResponse(prior["receipt"])
            if body["expected_revision"] != s["revision"]:
                return _err(409, "revision_conflict")
            s["revision"] += 1
            receipt = {"revision": s["revision"], "effect_receipt": f"rcpt-{key[:8]}",
                       "result": {"ok": True, "type": body["action"]["type"]}, "state_digest": "sha256:" + "2" * 64}
            s["actions"][key] = {"payload": body["action"], "receipt": receipt}
            world.bank_effects[(tenant, key)] += 1
        if mode == "lose_response":  # effect applied, answer lost
            return _err(504, "gateway_timeout")
        return JSONResponse(receipt)

    @app.get(sb + "/sessions/{sid}/actions/{key}")
    def sb_read(sid: str, key: str, request: Request) -> JSONResponse:
        claims = broker_gate(request, "sandbox")
        if isinstance(claims, JSONResponse):
            return claims
        s = own(claims, sid)
        got = s["actions"].get(key) if s else None
        return _err(404, "action_not_found") if got is None else JSONResponse(got["receipt"])

    @app.post(sb + "/sessions/{sid}/close")
    async def sb_close(sid: str, request: Request) -> JSONResponse:
        claims = broker_gate(request, "sandbox")
        if isinstance(claims, JSONResponse):
            return claims
        s = own(claims, sid)
        if s is None:
            return _err(404, "session_not_found")
        body = json.loads(await request.body() or b"{}")
        s["closed"] = body.get("reason")
        return JSONResponse({"state": "closed", "final_state_ref": f"final:{sid}"})

    # ---- scripted OpenAI-compatible chat endpoint (the model double) ---------------------------------------
    @app.post("/llm/v1/chat/completions")
    async def chat(request: Request) -> JSONResponse:
        body = json.loads(await request.body() or b"{}")
        msgs = body.get("messages", [])
        system = "\n".join(str(m.get("content", "")) for m in msgs if m.get("role") == "system")
        user = "\n".join(str(m.get("content", "")) for m in msgs if m.get("role") == "user")
        answer = world.llm_answer(system, user)
        if answer is None:
            return _err(500, "unscripted_model_call")
        text = json.dumps(answer)
        return JSONResponse({"id": "chatcmpl-e2e", "object": "chat.completion", "created": int(time.time()),
                             "model": body.get("model", "scripted"),
                             "choices": [{"index": 0, "finish_reason": "stop",
                                          "message": {"role": "assistant", "content": text}}],
                             "usage": {"prompt_tokens": max(1, len(user) // 4), "completion_tokens": len(text) // 4,
                                       "total_tokens": max(1, len(user) // 4) + len(text) // 4}})

    # ---- admin channel (driver only) --------------------------------------------------------------------------
    ing = ingest.state.ingest if ingest is not None else None

    @app.get("/_e2e/state")
    def state() -> JSONResponse:
        snap = world.snapshot()
        if ing is not None:
            snap["ingest"] = {"batches": len(ing.batches), "events": len(ing.ledger),
                              "batch_summaries": [{"scan_mode": b["scan_mode"], "source_id": b["source_id"],
                                                   "tenant_id": b["tenant_id"], "n": len(b["events"]),
                                                   "digest": b["batch_digest"],
                                                   "runs": sorted({e["source_run_ref"] for e in b["events"]
                                                                   if e.get("source_run_ref")})}
                                                  for b in ing.batches],
                              "verification_receipts": list(ing.verification_receipts),
                              "cursors": {f"{k[0]}|{k[1]}": v for k, v in ing.cursors.items()},
                              "auth_log": list(ing.auth_log), "artifacts": len(ing.artifacts),
                              "event_keys": [list(k) for k in ing.ledger]}
        return JSONResponse(snap)

    @app.get("/_e2e/info")
    def info() -> JSONResponse:
        return JSONResponse({"double": True, "pieces": ["control-api:fixture", "lab-broker:fixture", "llm:scripted",
                                                         "bank:fixture"] + (["ingest:fixture"] if ing else [])})

    @app.post("/_e2e/config")
    async def config(request: Request) -> JSONResponse:
        cfg = json.loads(await request.body() or b"{}")
        with world.lock:
            if "deny_operations" in cfg:
                world.deny_operations = set(cfg["deny_operations"])
            for route, modes in cfg.get("faults", {}).items():
                world.faults[route] = list(modes)
            if "llm_rules" in cfg:
                new = [{"repeat_last": False, **r} for r in cfg["llm_rules"]]
                world.llm_rules = new if cfg.get("llm_replace") else world.llm_rules + new
            for item in cfg.get("artifacts", []):
                world.put_artifact(item["tenant"], item["id"], item["content"])
            for item in cfg.get("wiki", []):
                world.wiki[(item["tenant"], item["path"])] = item["content"]
            if "ingest_fail_next" in cfg and ing is not None:
                ing.fail_next.extend((c, {}) for c in cfg["ingest_fail_next"])
        return JSONResponse({"ok": True})

    if ingest is not None:  # exporter -> control-api observation ingest + broker artifact upload
        app.router.routes.extend(r for r in ingest.router.routes if getattr(r, "path", "").startswith("/internal/"))
    return app
