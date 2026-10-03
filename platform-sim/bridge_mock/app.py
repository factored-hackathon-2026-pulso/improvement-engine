"""Faithful bridge wire mock (CAP-53, plan 17.3.1 / D.1-D.2): `/internal/v1/*` of `pulso-core-runtime` as a real HTTP
process with in-memory state, no agent_core, no Pulso adapter code. Label: `runtime_profile=contract_mock`.

Reproduces: service-JWT auth (EdDSA, `typ=JWT`, `kid`, `exp<=5min`, `jti` replay, per-route `purpose`, tenant claim ==
body tenant), private error shape `{schema_version, code, retryable, trace_id, details}`, single-flight invoke keyed by
`(tenant, Idempotency-Key)` with digest conflicts, the receipt state machine
(`sent -> binding_confirmed -> terminal_ok|terminal_failed`, `unknown`, `manual_reconcile`), the binding callback
(denied => zero effects), death before binding / after binding / after Core commit, `bridge_busy` (429, retryable),
`run_not_found`, cross-tenant invisibility, release pin errors and the stage fact envelope.

Not simulated (documented subset): real Flow execution, real registry validation (dry-run checks limits/kind only), real
broker. Fault injection lives only under `/_sim/*`. Request/response bodies are validated against `schemas/*.json`."""

from __future__ import annotations

import asyncio
import hashlib
import json
import uuid
from dataclasses import dataclass, field
from datetime import timedelta
from pathlib import Path
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, Response
from jsonschema import Draft202012Validator
from referencing import Registry, Resource
from starlette.exceptions import HTTPException as StarletteHTTPException

from bridge_mock import service_jws
from registry_mock import jws as core_jws
from registry_mock.app import KINDS
from registry_mock.sim_common import CONTRACT_VERSION, PIN_SHA, SimClock

SCHEMA_DIR = Path(__file__).resolve().parent / "schemas"
PROFILE = "contract_mock"
STAGES = ("scout", "verifier", "builder_design", "writer")
STAGE_FACT = {"scout": "pulso_hypotheses", "verifier": "pulso_verification", "builder_design": "pulso_change_spec",
              "writer": "pulso_writer_receipts"}
ROUTE_PURPOSE = {"invoke": "task_invoke", "read": "task_read", "alias": "state_read", "dryrun": "authoring_dry_run",
                 "version": "version_read", "issue": "credential_issue"}
MAX_INPUT_BYTES = 256 * 1024
MAX_CHANGES, MAX_ENTITY_BYTES = 50, 262_144
ISSUABLE_ROLES = frozenset({"constructor"})  # never a human or approval role
CREDENTIAL_TTL = timedelta(minutes=15)
STATUS_RETRYABLE = {"bridge_busy": True, "core_unavailable": True}


def _load_schemas() -> tuple[dict[str, dict], Registry]:
    schemas: dict[str, dict] = {}
    registry = Registry()
    for p in sorted(SCHEMA_DIR.glob("*.schema.json")):
        doc = json.loads(p.read_text(encoding="utf-8"))
        schemas[p.name.removesuffix(".schema.json")] = doc
        registry = registry.with_resource(p.name, Resource.from_contents(doc))
    return schemas, registry


SCHEMAS, REGISTRY = _load_schemas()


def schema_errors(name: str, instance: Any) -> list[str]:
    validator = Draft202012Validator(SCHEMAS[name], registry=REGISTRY)
    return sorted({"/".join(str(x) for x in e.absolute_path) or "(root)" for e in validator.iter_errors(instance)})


class BridgeError(Exception):
    def __init__(self, status: int, code: str, details: dict[str, Any] | None = None) -> None:
        super().__init__(code)
        self.status, self.code, self.details = status, code, details or {}


class Die(Exception):
    """Simulated process death: the connection is cut mid-response; state already committed stays committed."""


class BrokenResponse(Response):
    async def __call__(self, scope: Any, receive: Any, send: Any) -> None:
        await send({"type": "http.response.start", "status": 200,
                    "headers": [(b"content-type", b"application/json"), (b"content-length", b"100")]})
        await send({"type": "http.response.body", "body": b"{", "more_body": False})


@dataclass
class Run:
    core_run_id: str
    tenant_id: str
    stage: str
    outcome: str
    facts: dict[str, dict[str, Any]]
    missing_fact: bool = False


@dataclass
class Invocation:
    tenant_id: str
    key: str
    digest: str
    job_id: str
    stage: str
    attempt: int
    release_id: str
    state: str = "sent"
    core_run_id: str | None = None
    error_code: str | None = None
    committed_run: Run | None = None


def _sha(value: Any) -> str:
    return hashlib.sha256(service_jws.canonical(value)).hexdigest()


def stage_fact_value(stage: str) -> tuple[Any, str]:
    ev = [{"id": "art-1", "digest": "0" * 64, "media_type": "application/json"}]
    values = {
        "scout": ({"schema_version": "1", "hypotheses": [{"id": "h1", "statement": "contract fixture", "mechanism": "n/a",
                                                         "evidence_refs": ev, "counterevidence_refs": [],
                                                         "missing_evidence": [], "next_queries": []}]}, "agent"),
        "verifier": ({"schema_version": "1", "assessments": [{"hypothesis_id": "h1", "verdict": "inconclusive",
                                                              "evidence_refs": ev, "counterevidence_refs": [],
                                                              "limitations": ["contract fixture"]}]}, "agent"),
        "builder_design": ({"schema_version": "1", "change_spec": {"type": "do_nothing"}, "rationale": "contract fixture",
                            "evidence_refs": ev, "alternatives": []}, "agent"),
        "writer": ({"schema_version": "1", "proposal_id": "proposal-0001", "rev": 1, "candidate_hash": None,
                    "native_evaluation": None, "write_receipts": [], "state": "confirmed"}, "tool"),
    }
    return values[stage]


class State:
    """Everything mutable. `/_sim/reset` builds a fresh one; `/_sim/restart` only changes the instance id."""

    def __init__(self) -> None:
        self.clock = SimClock()
        self.instance_n = 1
        self.invocations: dict[tuple[str, str], Invocation] = {}
        self.runs: dict[str, Run] = {}
        self.bindings: list[dict[str, Any]] = []
        self.effects = {"lab_queries": 0, "model_calls": 0, "registry_writes": 0}
        self.releases: dict[str, dict[str, Any]] = {
            "rel-mock-0001": {"status": "active", "agents": {f"pulso-{s.replace('_', '-')}": "1.0.0" for s in STAGES}}}
        self.aliases: dict[tuple[str, str], str | None] = {}
        self.outcomes: list[str] = []
        self.binding_script: list[str] = []
        self.faults: list[dict[str, Any]] = []
        self.max_inflight = 8
        self.inflight = 0
        self.seen_jti: set[str] = set()
        self.run_n = 0

    @property
    def instance_id(self) -> str:
        return f"bridge-mock-{self.instance_n}"


def problem(status: int, code: str, trace_id: str, details: dict[str, Any] | None = None) -> JSONResponse:
    body = {"schema_version": "1", "code": code, "retryable": STATUS_RETRYABLE.get(code, False), "trace_id": trace_id,
            "details": details or {}}
    return JSONResponse(body, status_code=status)


def create_app() -> Any:
    holder = {"st": State()}
    app = FastAPI(title="bridge-mock", version="1.0.0", docs_url=None, redoc_url=None, openapi_url="/openapi.json")

    def st() -> State:
        return holder["st"]

    @app.middleware("http")
    async def _trace(request: Request, call_next):  # type: ignore[no-untyped-def]
        request.state.trace_id = uuid.uuid4().hex
        return await call_next(request)

    def tid(request: Request) -> str:
        return str(getattr(request.state, "trace_id", "unknown"))

    @app.exception_handler(BridgeError)
    async def _bridge(request: Request, exc: BridgeError) -> Response:
        return problem(exc.status, exc.code, tid(request), exc.details)

    @app.exception_handler(Die)
    async def _die(request: Request, exc: Die) -> Response:
        return BrokenResponse()

    @app.exception_handler(StarletteHTTPException)
    async def _http(request: Request, exc: StarletteHTTPException) -> Response:
        code = {404: "not_found", 405: "method_not_allowed"}.get(exc.status_code, "internal_error"
                                                                  if exc.status_code >= 500 else "invalid_request")
        return problem(exc.status_code, code, tid(request))

    @app.exception_handler(Exception)
    async def _unexpected(request: Request, exc: Exception) -> Response:
        return problem(500, "internal_error", tid(request))

    # --- helpers ---
    def authenticate(request: Request, route: str) -> dict[str, Any]:
        scheme, _, rest = request.headers.get("authorization", "").strip().partition(" ")
        if scheme.lower() != "bearer" or not rest.strip():
            raise BridgeError(401, "credentials_invalid")
        s = st()
        try:
            claims = service_jws.verify(rest.strip(), s.clock.now())
        except Exception:  # signature, header, kid, claims, exp: all the same outwards
            raise BridgeError(401, "credentials_invalid") from None
        if claims["jti"] in s.seen_jti:  # the receiver consumes (issuer, jti) atomically before dispatch
            raise BridgeError(401, "credentials_invalid", {"reason": "replay"})
        s.seen_jti.add(claims["jti"])
        if claims["purpose"] != ROUTE_PURPOSE[route]:
            raise BridgeError(403, "forbidden", {"reason": "purpose"})
        return claims

    async def json_body(request: Request) -> Any:
        try:
            return json.loads(await request.body())
        except ValueError:
            raise BridgeError(400, "invalid_request", {"reason": "not json"}) from None

    def check_schema(name: str, body: Any) -> None:
        bad = schema_errors(name, body)
        if bad:
            raise BridgeError(400, "invalid_request", {"paths": bad})

    def j(value: Any, status: int = 200) -> Response:
        return Response(json.dumps(value, ensure_ascii=False), status_code=status, media_type="application/json")

    def envelope(run: Run) -> dict[str, Any]:
        return {"schema_version": "1", "core_run_id": run.core_run_id, "status": run.outcome, "outcome": run.outcome,
                "facts": run.facts, "output_digest": _sha({k: v["digest"] for k, v in sorted(run.facts.items())}),
                "runtime_profile": PROFILE}

    def receipt(inv: Invocation, request: Request) -> dict[str, Any]:
        run = inv.committed_run if inv.state.startswith("terminal") else None
        ok = run is not None and not run.missing_fact
        return {"schema_version": "1", "tenant_id": inv.tenant_id, "job_id": inv.job_id, "stage": inv.stage,
                "attempt": inv.attempt, "idempotency_key": inv.key, "request_digest": inv.digest, "state": inv.state,
                "core_run_id": inv.core_run_id, "bridge_instance_id": st().instance_id, "release_id": inv.release_id,
                "outcome": run.outcome if ok and run else None, "result": envelope(run) if ok and run else None,
                "error_code": inv.error_code, "runtime_profile": PROFILE, "trace_id": tid(request)}

    def reconcile(inv: Invocation) -> None:
        """Replay rules: never re-enter the engine; promote what the persisted facts already prove."""
        if inv.state == "sent" and inv.core_run_id is None:
            inv.state = "manual_reconcile"  # no binding and `sent`
        elif inv.state == "binding_confirmed":
            run = inv.committed_run
            if run is None:
                inv.state = "unknown"  # binding without a Core commit: effects may exist, absence is unproven
            elif run.missing_fact:
                inv.state, inv.error_code = "terminal_failed", "output_missing"
            else:
                inv.state = "terminal_ok" if run.outcome == "completed" else "terminal_failed"

    # --- routes ---
    @app.post("/internal/v1/core-tasks/invoke")
    async def invoke(request: Request) -> Response:
        claims = authenticate(request, "invoke")
        body = await json_body(request)
        key = request.headers.get("idempotency-key", "")
        if not 1 <= len(key) <= 255:
            raise BridgeError(400, "invalid_request", {"reason": "Idempotency-Key"})
        if isinstance(body, dict) and "stage" in body and body["stage"] not in STAGES:
            raise BridgeError(400, "stage_unknown")
        check_schema("CoreTaskInvocation", body)
        try:
            too_big = len(service_jws.canonical(body["input"])) > MAX_INPUT_BYTES
            digest = service_jws.request_digest(body)
        except ValueError:
            raise BridgeError(400, "invalid_request", {"reason": "non-integer number"}) from None
        if too_big:
            raise BridgeError(413, "input_too_large")
        if digest != body["request_digest"]:
            raise BridgeError(400, "request_digest_invalid")
        if claims["tenant_id"] != body["tenant_id"]:
            raise BridgeError(403, "tenant_mismatch")
        s = st()
        existing = s.invocations.get((body["tenant_id"], key))
        if existing is not None:
            if existing.digest != digest:
                raise BridgeError(409, "digest_conflict")
            reconcile(existing)
            return j(receipt(existing, request))
        if s.inflight >= s.max_inflight:
            raise BridgeError(429, "bridge_busy")
        # pre-pin checks (release exists, active, agent/version match)
        release = s.releases.get(body["release_id"])
        if release is None:
            raise BridgeError(409, "release_pin_unavailable")
        if release["status"] != "active":
            raise BridgeError(409, "release_revoked")
        if release["agents"].get(body["agent_id"]) != body["agent_version"]:
            raise BridgeError(409, "release_drift")
        fault = s.faults.pop(0) if s.faults else None
        s.inflight += 1
        try:
            if fault and fault["mode"] == "latency":
                await asyncio.sleep(float(fault.get("seconds", 1)))
            if fault and fault["mode"] == "status503":
                raise BridgeError(503, "core_unavailable")
            inv = Invocation(body["tenant_id"], key, digest, body["job_id"], body["stage"], body["attempt"],
                             body["release_id"])
            s.invocations[(inv.tenant_id, key)] = inv  # committed `sent` before the call
            if fault and fault["mode"] == "die_before_binding":
                raise Die()
            callback = s.binding_script.pop(0) if s.binding_script else "ok"
            if callback != "ok":  # binding refused: toda query, model or write is denied; absence of effect unproven
                inv.state = "manual_reconcile"
                raise BridgeError(403, "binding_failed", {"callback": callback})
            s.run_n += 1
            inv.core_run_id = f"core-run-{s.run_n:04d}"
            inv.state = "binding_confirmed"
            s.bindings.append({
                "schema_version": "1", "tenant_id": inv.tenant_id, "job_id": inv.job_id, "command_key": key,
                "request_digest": digest, "attempt": inv.attempt, "core_run_id": inv.core_run_id,
                "bridge_instance_id": s.instance_id, "task_binding_ref": f"tbr-{s.run_n:04d}"})
            if fault and fault["mode"] == "die_after_binding":
                raise Die()
            # --- engine: effects happen only after a confirmed binding ---
            s.effects["model_calls"] += 1
            if inv.stage in ("scout", "verifier", "builder_design"):
                s.effects["lab_queries"] += 1
            if inv.stage == "writer":
                s.effects["registry_writes"] += 1
            outcome = s.outcomes.pop(0) if s.outcomes else "completed"
            facts: dict[str, dict[str, Any]] = {}
            if outcome == "completed":
                value, source = stage_fact_value(inv.stage)
                facts = {STAGE_FACT[inv.stage]: {"value": value, "source_kind": source, "digest": _sha(value)}}
            run = Run(inv.core_run_id, inv.tenant_id, inv.stage, "failed" if outcome == "failed" else "completed", facts,
                      missing_fact=outcome == "completed_without_fact")
            s.runs[run.core_run_id] = run
            inv.committed_run = run  # Core commits the run and its idempotency record together
            if fault and fault["mode"] == "die_after_commit":
                raise Die()
            reconcile(inv)
            if run.missing_fact:
                raise BridgeError(422, "output_missing")
            return j(receipt(inv, request))
        finally:
            s.inflight -= 1

    @app.get("/internal/v1/core-tasks/{core_run_id}")
    async def read_run(request: Request, core_run_id: str) -> Response:
        claims = authenticate(request, "read")
        run = st().runs.get(core_run_id)
        if run is None or run.tenant_id != claims["tenant_id"]:  # a foreign tenant's run id is not visible
            raise BridgeError(404, "run_not_found")
        if run.missing_fact:
            raise BridgeError(422, "output_missing")
        return j(envelope(run))

    @app.get("/internal/v1/core-state/aliases/{agent_id}/{alias}")
    async def alias_state(request: Request, agent_id: str, alias: str) -> Response:
        authenticate(request, "alias")
        if alias not in ("staging", "prod"):
            raise BridgeError(404, "not_found")
        return j({"schema_version": "1", "agent_id": agent_id, "alias": alias,
                  "release_id": st().aliases.get((agent_id, alias)), "runtime_profile": PROFILE})

    @app.post("/internal/v1/core-authoring/dry-run")
    async def dry_run(request: Request) -> Response:
        claims = authenticate(request, "dryrun")
        body = await json_body(request)
        check_schema("CoreAuthoringDryRunRequest", body)
        if claims["tenant_id"] != body["tenant_id"]:
            raise BridgeError(403, "tenant_mismatch")
        violations: list[dict[str, Any]] = []
        if len(body["changes"]) > MAX_CHANGES:
            violations.append({"rule": "REG-LIMIT", "path": None,
                               "message": f"{len(body['changes'])} changes; max {MAX_CHANGES}"})
        else:
            for c in body["changes"]:
                where = f"{c['kind'][:40]}:{str(c['content'].get('id'))[:80]}"
                if c["kind"] not in KINDS:
                    violations.append({"rule": "REG-KIND", "path": where, "message": "unknown entity kind"})
                try:
                    size = len(service_jws.canonical(c["content"]))
                except ValueError:
                    size = 0
                if size > MAX_ENTITY_BYTES:
                    violations.append({"rule": "REG-LIMIT", "path": where, "message": f"{size} bytes; max {MAX_ENTITY_BYTES}"})
        cand = None if violations else _sha({"agent": body["agent_id"], "base": body["base_release_id"],
                                               "changes": body["changes"]})
        return j({"schema_version": "1", "valid": not violations, "candidate_hash": cand, "violations": violations,
                  "proposal_created": False, "runtime_profile": PROFILE})

    @app.get("/internal/v1/version")
    async def version(request: Request) -> Response:
        authenticate(request, "version")
        return j({"schema_version": "1", "agent_core_sha": PIN_SHA, "contracts_version": CONTRACT_VERSION,
                  "pulso_sha": "contract-mock", "image_digest": None, "bridge_instance_id": st().instance_id,
                  "runtime_profile": PROFILE})

    @app.post("/internal/v1/core-credentials/issue")
    async def issue(request: Request) -> Response:
        claims = authenticate(request, "issue")
        body = await json_body(request)
        check_schema("CoreCredentialIssueRequest", body)
        if claims["tenant_id"] != body["tenant_id"]:
            raise BridgeError(403, "tenant_mismatch")
        if body["role"] not in ISSUABLE_ROLES:
            raise BridgeError(403, "forbidden", {"reason": "role not issuable"})
        now = st().clock.now()
        exp = now + CREDENTIAL_TTL
        payload = core_jws.principal_payload(f"bot:{body['tenant_id']}:{body['role']}", "builder", [body["role"]],
                                             exp=exp, now=now)
        token = core_jws.sign({"alg": core_jws.ALG, "kid": core_jws.SIM_KID, "typ": core_jws.PRINCIPAL_TYP}, payload)
        return j({"schema_version": "1", "jws": token, "kid": core_jws.SIM_KID, "exp": service_jws.iso(exp),
                  "runtime_profile": PROFILE})  # returned, never stored or logged

    # --- /_sim/* control channel (absent from the real runtime; faults are OFF by default) ---
    @app.get("/_sim/info")
    def info() -> dict[str, str]:
        return {"pinned_sha": PIN_SHA, "contract_version": CONTRACT_VERSION, "runtime_profile": PROFILE, "level": "mock",
                "now": service_jws.iso(st().clock.now())}

    @app.post("/_sim/reset")
    def reset() -> dict[str, str]:
        holder["st"] = State()
        return {"status": "reset"}

    @app.post("/_sim/restart")
    def restart() -> dict[str, Any]:
        """Process restart: durable state (invocations, runs, bindings) survives; the instance id changes."""
        st().instance_n += 1
        st().inflight = 0
        return {"bridge_instance_id": st().instance_id}

    async def sim_json(request: Request) -> dict[str, Any]:
        data = await request.json()
        if not isinstance(data, dict):
            raise BridgeError(400, "invalid_request")
        return data

    @app.post("/_sim/fault")
    async def fault(request: Request) -> dict[str, int]:
        spec = await sim_json(request)
        if spec.get("mode") not in ("latency", "status503", "die_before_binding", "die_after_binding", "die_after_commit"):
            raise BridgeError(400, "invalid_request", {"reason": "unknown fault mode"})
        st().faults.append(spec)
        return {"queued": len(st().faults)}

    @app.post("/_sim/outcomes")
    async def outcomes(request: Request) -> dict[str, list[str]]:
        script = list((await sim_json(request))["script"])
        if any(x not in ("completed", "failed", "completed_without_fact") for x in script):
            raise BridgeError(400, "invalid_request", {"reason": "unknown outcome"})
        st().outcomes = script
        return {"script": script}

    @app.post("/_sim/binding")
    async def binding(request: Request) -> dict[str, list[str]]:
        script = list((await sim_json(request))["script"])
        if any(x not in ("ok", "binding_conflict", "digest_mismatch", "command_unknown", "unavailable", "timeout")
               for x in script):
            raise BridgeError(400, "invalid_request", {"reason": "unknown binding callback outcome"})
        st().binding_script = script
        return {"script": script}

    @app.post("/_sim/config")
    async def config(request: Request) -> dict[str, int]:
        data = await sim_json(request)
        if "max_inflight" in data:
            st().max_inflight = int(data["max_inflight"])
        return {"max_inflight": st().max_inflight}

    @app.post("/_sim/releases")
    async def releases(request: Request) -> dict[str, str]:
        d = await sim_json(request)
        st().releases[d["release_id"]] = {"status": d.get("status", "active"), "agents": dict(d.get("agents", {}))}
        return {"release_id": d["release_id"]}

    @app.post("/_sim/aliases")
    async def aliases(request: Request) -> dict[str, str]:
        d = await sim_json(request)
        st().aliases[(d["agent_id"], d["alias"])] = d.get("release_id")
        return {"status": "set"}

    @app.get("/_sim/effects")
    def effects() -> dict[str, int]:
        return dict(st().effects)

    @app.get("/_sim/bindings")
    def bindings() -> list[dict[str, Any]]:
        return list(st().bindings)

    @app.get("/_sim/invocations")
    def invocations() -> list[dict[str, Any]]:
        return [{"tenant_id": i.tenant_id, "key": i.key, "state": i.state, "core_run_id": i.core_run_id,
                 "error_code": i.error_code} for i in st().invocations.values()]

    return app
