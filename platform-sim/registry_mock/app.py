"""Faithful registry wire mock (CAP-52): a real HTTP process, in-memory state, no SQL, no agent_core, no Pulso adapter.

Reproduces the 18 routes of `/v1/registry` (16 + the N-02 alias/versions reads at agent-core 894fa65), EdDSA JWS auth, the 12 registry error codes, `application/problem+json`
envelopes, the mandatory `Idempotency-Key` on publish, state machine, CAS, limits and quotas with an injectable
clock. Candidate validation is a documented SUBSET of the real rules (REG-KIND, REG-VERSION, REG-VERSION-TAKEN,
REG-LIMIT); everything else is the a2 level's job. Fault injection lives only under `/_sim/*` (default off).
Evaluation is `contract_fixture`: programmable (pass/fail/failed_infra/timeout) and never simulates improvement.
"""

from __future__ import annotations

import asyncio
import copy
import os
import hashlib
import json
import re
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Annotated, Any, Literal

from fastapi import APIRouter, FastAPI, Header, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse, Response
from pydantic import BaseModel, ConfigDict, Field, model_validator
from starlette.exceptions import HTTPException as StarletteHTTPException

from registry_mock import jws
from registry_mock.sim_common import (
    AGENT_ID,
    BASE_RELEASE_ID,
    CONTRACT_VERSION,
    PIN_SHA,
    SimClock,
    fixtures_digest,
)

WIRE = Path(os.environ.get("PULSO_WIRE_DIR") or Path(__file__).resolve().parents[2] / "core-bridge" / "wire" / f"agent_core@{PIN_SHA[:7]}")
KINDS = frozenset({"agent", "flow", "policy", "template", "prompt", "tool", "decision_model", "model_profile",
                   "language_detection", "injection_ruleset", "knowledge_snapshot", "eval_suite"})
SETTINGS_KIND = "release_settings"  # N-07 reserved draft kind (not an entity)
MAX_INPUT_CHARS_CEILING = 100_000  # 894fa65: a proposal cannot disable the input-length guard
SETTINGS_FIELDS = ("interrupts", "language_detection", "injection_ruleset", "max_input_chars")
ERROR_STATUS = {
    "validation_failed": 422, "gate_failed": 409, "proposal_stale": 409, "candidate_changed": 409,
    "illegal_transition": 409, "forbidden_role": 403, "step_up_required": 403, "integrity_error": 500,
    "not_found": 404, "loosening_not_accepted": 409, "idempotency_conflict": 409, "quota_exceeded": 429,
}
PROBLEM_TITLES = {
    "credentials_invalid": "Credenciales inválidas", "principal_expired": "Principal vencido",
    "not_found": "Recurso inexistente", "invalid_request": "Solicitud inválida", "internal_error": "Error interno",
}
ID_RE = re.compile(r"^[a-z0-9][a-z0-9_/-]*$")
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")


@dataclass(frozen=True)
class Limits:
    max_changes: int = 50
    max_entity_bytes: int = 262_144
    max_flow_nodes: int = 200
    max_suite_scenarios: int = 200  # EvalSuite.scenarios max_length: a schema violation (REG-SCHEMA), not REG-LIMIT
    proposals_per_day: int = 10
    evals_per_proposal: int = 20
    window: timedelta = timedelta(hours=24)


class RegistryError(Exception):
    def __init__(self, code: str, detail: str = "", payload: Any = None) -> None:
        super().__init__(code)
        self.code, self.detail, self.payload = code, detail, payload


class CredentialsInvalid(Exception):
    pass


class PrincipalExpired(Exception):
    pass


# --- request models (same strictness as the real API: body top level lenient, drafts extra=forbid) ----------

class VersionDocs(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)
    description: str = Field(min_length=1, max_length=4000)
    rationale: str = Field(max_length=4000)
    changelog: str = Field(max_length=8000)


class EntityDraft(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)
    kind: str
    content: dict[str, Any]
    docs: VersionDocs

    @model_validator(mode="after")
    def _has_identity(self) -> "EntityDraft":
        if self.kind == SETTINGS_KIND:  # N-07: the reserved draft has no identity of its own
            return self
        if not isinstance(self.content.get("id"), str) or not isinstance(self.content.get("version"), str):
            raise ValueError("content needs id and version as text")
        return self


class _Create(BaseModel):
    agent_id: str
    origin: Literal["manual", "builder_chat", "auto_detect", "import"] = "manual"
    title: str


class _Draft(BaseModel):
    expected_rev: int
    changes: list[EntityDraft]


class _Evaluate(BaseModel):
    suite_id: str
    suite_version: str | None = None


class _Approve(BaseModel):
    candidate_hash: str
    accept_yardstick_loosened: bool = False


class _Reason(BaseModel):
    reason: str


class _Promote(BaseModel):
    release_id: str
    reason: str = ""


# --- helpers -----------------------------------------------------------------------------------------------

def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def semver(v: str) -> tuple[int, int, int] | None:
    m = SEMVER.fullmatch(v)
    return (int(m[1]), int(m[2]), int(m[3])) if m else None


def ref_str(ref: tuple[str, str, str]) -> str:
    return f"{ref[0]}:{ref[1]}@{ref[2]}"


def ref_json(ref: tuple[str, str, str]) -> dict[str, str]:
    return {"kind": ref[0], "id": ref[1], "version": ref[2]}


def violation(rule: str, message: str, path: str | None = None) -> dict[str, Any]:
    return {"rule": rule, "path": path, "flow": None, "node_id": None, "message": message}


@dataclass
class Principal:
    type: str
    id: str | None
    roles: list[str]
    attrs: dict[str, str]
    auth_level: str
    exp: datetime


class _PrincipalModel(BaseModel):
    model_config = ConfigDict(extra="forbid")
    type: Literal["customer", "advisor", "builder", "service"]
    id: str | None = Field(default=None, min_length=1)
    roles: list[str] = []
    scopes: list[str] = []
    attrs: dict[str, str] = {}
    auth: dict[str, Any]
    exp: datetime


def interrupt_norm(i: dict[str, Any]) -> dict[str, Any]:
    """An interrupt as `ReleaseDetail` serves it: every optional field present (`locked` defaults to false)."""
    return {"id": i["id"], "priority": i["priority"], "action": copy.deepcopy(i["action"]),
            "signal_policy": i.get("signal_policy"), "locked": bool(i.get("locked", False))}


def _same_action(a: dict[str, Any], b: dict[str, Any]) -> bool:
    if a["action"].get("type") == "start_flow" and b["action"].get("type") == "start_flow":
        return (a["action"].get("flow") or {}).get("id") == (b["action"].get("flow") or {}).get("id")
    return a["action"] == b["action"]


def locked_interrupt_violations(base: dict[str, Any] | None, wanted: list[dict[str, Any]] | None) -> list[dict[str, Any]]:
    """REG-LOCKED: `wanted` replaces the base interrupts, but the base's `locked` ones must stay, with the same action,
    at least the same priority and `locked`. `wanted is None` inherits the base."""
    if base is None or wanted is None:
        return []
    kept = {i["id"]: i for i in wanted}
    found: list[dict[str, Any]] = []
    for locked in (i for i in base.get("interrupts", []) if i.get("locked")):
        now = kept.get(locked["id"])
        if now is None:
            found.append(violation("REG-LOCKED", f"la interrupcion {str(locked['id'])[:80]} es de plataforma: no se quita",
                                   SETTINGS_KIND))
        elif not now.get("locked") or now["priority"] < locked["priority"] or not _same_action(now, locked):
            found.append(violation("REG-LOCKED", f"la interrupcion {str(locked['id'])[:80]} es de plataforma: no se le baja "
                                   "la prioridad, no se le cambia la accion ni se le quita el bloqueo", SETTINGS_KIND))
    return found


def settings_schema_problems(content: dict[str, Any]) -> list[str]:
    """Subset of `ReleaseSettings` (extra=forbid): the field names, `max_input_chars` in (0, ceiling] and the shape of
    each interrupt (id, int priority, escalate | start_flow action). Returns schema messages."""
    out: list[str] = []
    for key in sorted(content):
        if key not in SETTINGS_FIELDS:
            out.append(f"{key}: Extra inputs are not permitted")
    mic = content.get("max_input_chars")
    if mic is not None:
        if isinstance(mic, bool) or not isinstance(mic, int):
            out.append("max_input_chars: Input should be a valid integer")
        elif mic <= 0:
            out.append("max_input_chars: Input should be greater than 0")
        elif mic > MAX_INPUT_CHARS_CEILING:
            out.append(f"max_input_chars: Input should be less than or equal to {MAX_INPUT_CHARS_CEILING}")
    for key in ("language_detection", "injection_ruleset"):
        if content.get(key) is not None and not isinstance(content[key], str):
            out.append(f"{key}: Input should be a valid string")
    interrupts = content.get("interrupts")
    if interrupts is not None:
        if not isinstance(interrupts, list):
            out.append("interrupts: Input should be a valid list")
        else:
            for n, i in enumerate(interrupts):
                ok = (isinstance(i, dict) and isinstance(i.get("id"), str) and isinstance(i.get("priority"), int)
                      and isinstance(i.get("action"), dict) and i["action"].get("type") in ("escalate", "start_flow")
                      and isinstance(i.get("locked", False), bool))
                if not ok:
                    out.append(f"interrupts.{n}: Input should be a valid Interrupt")
    return out


def settings_draft(drafts: list[Any]) -> list[Any]:
    return [d for d in drafts if d.kind == SETTINGS_KIND]


def _slot_reads(flow: dict[str, Any]) -> dict[str, str]:
    """Slots a flow reads (`slots.<name>` anywhere in its content) -> first node that reads it (subset of AG-04)."""
    found: dict[str, str] = {}

    def walk(value: Any, node_id: str) -> None:
        if isinstance(value, dict):
            for v in value.values():
                walk(v, node_id)
        elif isinstance(value, list):
            for v in value:
                walk(v, node_id)
        elif isinstance(value, str):
            m = re.match(r"slots\.([A-Za-z0-9_]+)", value)
            if m:
                found.setdefault(m.group(1), node_id)

    for node in flow.get("nodes") or []:
        walk(node, str(node.get("id")))
    return found


def ag04_violations(flow: dict[str, Any], agent: dict[str, Any]) -> list[dict[str, Any]]:
    """AG-04: the flow reads `slots.X` and nothing can leave it `validated`: only a `collect` node, the agent's
    `input_schema` (task agents) or the `accepts` contract of a transfer write validated slots."""
    writable = {(n.get("config") or {}).get("slot") for n in flow.get("nodes") or [] if n.get("type") == "collect"}
    writable |= set(agent.get("input_schema") or {})
    writable |= set(((agent.get("accepts") or {}).get("slots")) or {})
    label = f"{flow.get('id')}@{flow.get('version')}"[:160]
    out = []
    for name, node_id in sorted(_slot_reads(flow).items()):
        if name not in writable:
            out.append({**violation("AG-04", f"el flow lee slots.{name} y {agent.get('id')}@{agent.get('version')} no tiene "
                                    "como dejarlo validado (un nodo collect, input_schema o accepts)"),
                        "flow": label, "node_id": node_id})
    return out


@dataclass
class Version:
    ref: tuple[str, str, str]
    content: dict[str, Any]
    content_hash: str
    docs: dict[str, str]
    created_by: str
    created_at: datetime


@dataclass
class Release:
    release_id: str
    agent_id: str
    refs: list[tuple[str, str, str]]
    base_release_id: str | None
    proposal_id: str | None
    published_by: str
    published_at: datetime
    eval_suite_refs: list[tuple[str, str, str]] = field(default_factory=list)
    status: str = "active"
    # N-03: release-level settings served by `ReleaseDetail`. A published release inherits its base's (N-07
    # `release_settings` drafts are not simulated by the mock: a2/real are the authority for them).
    settings: dict[str, Any] = field(default_factory=dict)


@dataclass
class ProposalRec:
    proposal_id: str
    agent_id: str
    origin: str
    state: str
    rev: int
    base_release_id: str | None
    title: str
    created_by: str
    candidate_hash: str | None
    updated_at: datetime
    created_at: datetime


class State:
    """All mutable state of one simulated registry; `/_sim/reset` builds a fresh one."""

    def __init__(self, limits: Limits) -> None:
        self.limits = limits
        self.clock = SimClock()
        self.eval_script: list[str] = []
        self.proposals: dict[str, ProposalRec] = {}
        self.changes: dict[str, list[EntityDraft]] = {}
        self.versions: dict[tuple[str, str, str], Version] = {}
        self.releases: dict[str, Release] = {}
        self.aliases: dict[tuple[str, str], str] = {}
        self.evals: list[dict[str, Any]] = []
        self.approvals: list[dict[str, Any]] = []
        self.publish_keys: dict[str, tuple[str, str]] = {}
        self.counters: dict[str, int] = {}
        self._seed()

    def new_id(self, kind: str) -> str:
        self.counters[kind] = self.counters.get(kind, 0) + 1
        return f"{kind}-{self.counters[kind]:04d}"

    def _seed(self) -> None:
        """Seed = the registry-demo release as recorded by gen-wire from the pinned agent-core (real hashes)."""
        vectors = json.loads((WIRE / "golden" / "hash_vectors.json").read_text(encoding="utf-8"))
        refs: list[tuple[str, str, str]] = []
        for e in vectors["entities"]:
            kind, rest = e["ref"].split(":", 1)
            eid, version = rest.rsplit("@", 1)
            ref = (kind, eid, version)
            refs.append(ref)
            self.versions[ref] = Version(ref, e["normalized_dump_json"], e["content_hash"],
                                         {"description": "Importado desde YAML", "rationale": "semilla",
                                          "changelog": ""}, "root", self.clock.now())
        detail = vectors["release_detail"]  # recorded by gen-wire from a2 at the pin (N-03 fields)
        settings = {k: detail[k] for k in ("interrupts", "language_detection", "injection_ruleset", "max_input_chars")}
        self.releases[vectors["release_id"]] = Release(vectors["release_id"], AGENT_ID, sorted(refs, key=ref_str),
                                                       None, None, "root", self.clock.now(),
                                                       settings=settings)
        assert vectors["release_id"] == BASE_RELEASE_ID
        for alias in ("staging", "prod"):
            self.aliases[(AGENT_ID, alias)] = BASE_RELEASE_ID


def iso(value: datetime) -> str:
    return jws.iso(value)


# --- candidate (documented subset of the real rules) ----------------------------------------------------------

@dataclass
class Candidate:
    refs: dict[tuple[str, str], tuple[str, str, str]]
    new_versions: list[tuple[str, str, str]]
    auto_bumped: list[tuple[str, str, str]]
    drafts: dict[tuple[str, str, str], EntityDraft]
    hash: str
    derived_agent: dict[str, Any] | None = None
    settings: dict[str, Any] | None = None  # the `release_settings` draft content (N-07); None = inherit the base


def _mentioned_ids(value: Any, out: set[str]) -> None:
    """Every `id` that appears as a reference ({id, spec}) or as a bare string inside the given content."""
    if isinstance(value, dict):
        if isinstance(value.get("id"), str) and "spec" in value:
            out.add(value["id"])
        for v in value.values():
            _mentioned_ids(v, out)
    elif isinstance(value, list):
        for v in value:
            _mentioned_ids(v, out)
    elif isinstance(value, str):
        out.add(value)


def _ids_of(d: EntityDraft) -> set[str]:
    out: set[str] = set()
    for key, value in d.content.items():
        if key not in ("id", "version"):
            _mentioned_ids(value, out)
    return out


def build_candidate(st: State, p: ProposalRec) -> tuple[list[dict[str, Any]], Candidate | None]:
    drafts = st.changes.get(p.proposal_id, [])
    base = st.releases.get(p.base_release_id) if p.base_release_id else None
    base_versions = {(r[0], r[1]): r[2] for r in base.refs} if base else {}
    problems: list[dict[str, Any]] = []
    by_ref: dict[tuple[str, str, str], EntityDraft] = {}
    wanted_settings: dict[str, Any] | None = None
    sets = settings_draft(drafts)
    if len(sets) > 1:
        problems.append(violation("REG-DUPLICATE", "release_settings aparece dos veces en el borrador", SETTINGS_KIND))
    elif sets:
        schema = settings_schema_problems(sets[0].content)
        if schema:
            problems.append(violation("REG-SCHEMA", "el contenido no cumple el esquema: " + "; ".join(schema), SETTINGS_KIND))
        else:
            wanted_settings = sets[0].content
            if wanted_settings.get("interrupts") is not None:
                problems.extend(locked_interrupt_violations(base.settings if base else None, wanted_settings["interrupts"]))
    for d in drafts:
        if d.kind == SETTINGS_KIND:
            continue
        where = f"{d.kind}:{d.content['id']}"
        if d.kind not in KINDS:
            problems.append(violation("REG-KIND", f"unknown entity kind {d.kind[:40]}", where))
            continue
        ref = (d.kind, d.content["id"], d.content["version"])
        digest = sha256_hex(canonical(d.content))
        existing = st.versions.get(ref)
        if existing is not None and existing.content_hash != digest:
            problems.append(violation("REG-VERSION-TAKEN", f"{ref_str(ref)} already exists with other content", ref_str(ref)))
        elif existing is None:
            old = base_versions.get((d.kind, d.content["id"]))
            if old is not None:
                new_v, old_v = semver(ref[2]), semver(old)
                if new_v is None or old_v is None or new_v <= old_v:
                    problems.append(violation("REG-VERSION", f"version {ref[2]} must be greater than {old}", where))
        if len(canonical(d.content)) > st.limits.max_entity_bytes:
            problems.append(violation("REG-LIMIT", f"{where} exceeds {st.limits.max_entity_bytes} bytes", where))
        if d.kind == "flow" and len(d.content.get("nodes") or []) > st.limits.max_flow_nodes:
            problems.append(violation("REG-LIMIT", f"flow has more than {st.limits.max_flow_nodes} nodes", where))
        scenarios = d.content.get("scenarios")
        if d.kind == "eval_suite" and isinstance(scenarios, list) and len(scenarios) > st.limits.max_suite_scenarios:
            problems.append(violation(
                "REG-SCHEMA", f"el contenido no cumple el esquema: scenarios: List should have at most "
                f"{st.limits.max_suite_scenarios} items after validation, not {len(scenarios)}", f"eval_suite:{ref[1]}"))
        by_ref[ref] = d
    mentioned: set[str] = set()
    for d in by_ref.values():
        _mentioned_ids(d.content, mentioned)
    for ref, d in by_ref.items():
        if ref[0] not in ("agent", "eval_suite", "flow") and (ref[0], ref[1]) not in base_versions                 and not any(ref[1] in _ids_of(x) for x in by_ref.values() if x is not d):
            problems.append(violation("REG-UNREFERENCED", f"{ref_str(ref)} has no consumer in the candidate",
                                      f"{ref[0]}:{ref[1]}"))
    agent_ref = next((r for r in by_ref if r[0] == "agent" and r[1] == p.agent_id), None)
    if agent_ref is not None and by_ref[agent_ref].content.get("input_schema") is not None \
            and by_ref[agent_ref].content.get("mode") != "task":
        problems.append(violation("REG-SCHEMA", "el contenido no cumple el esquema: : Value error, input_schema solo aplica a "
                                  "agentes de modo task", f"agent:{p.agent_id}"))
    # M1 AG-04 (subset): each drafted flow against the agent it will run under (the drafted one, else the base's)
    agent_content = by_ref[agent_ref].content if agent_ref else (
        st.versions[("agent", p.agent_id, base_versions[("agent", p.agent_id)])].content
        if ("agent", p.agent_id) in base_versions else None)
    if agent_content is not None and not any(pr["rule"] in ("REG-KIND", "REG-SCHEMA") for pr in problems):
        for ref, d in by_ref.items():
            if ref[0] == "flow":
                problems.extend(ag04_violations(d.content, agent_content))
    if problems:
        return problems, None
    refs = {k: (k[0], k[1], v) for k, v in base_versions.items()}
    new_versions: list[tuple[str, str, str]] = []
    for ref, d in by_ref.items():
        refs[(ref[0], ref[1])] = ref
        if ref not in st.versions:
            new_versions.append(ref)
    auto: list[tuple[str, str, str]] = []
    derived: dict[str, Any] | None = None
    has_agent_draft = any(r[0] == "agent" for r in by_ref)
    replaced = any(r[0] not in ("agent", "eval_suite") and (r[0], r[1]) in base_versions for r in by_ref)
    if not has_agent_draft and replaced and (("agent", p.agent_id) in refs):
        old_ref = refs[("agent", p.agent_id)]
        v = semver(old_ref[2]) or (1, 0, 0)
        new_ref = ("agent", p.agent_id, f"{v[0]}.{v[1]}.{v[2] + 1}")
        derived = copy.deepcopy(st.versions[old_ref].content)
        derived["version"] = new_ref[2]
        refs[("agent", p.agent_id)] = new_ref
        auto.append(new_ref)
        new_versions.append(new_ref)
    release_refs = sorted(r for k, r in refs.items() if k[0] != "eval_suite")
    digest = sha256_hex(canonical({"agent": p.agent_id, "base": p.base_release_id,
                                   "refs": [ref_str(r) for r in release_refs],
                                   "hashes": sorted(sha256_hex(canonical(d.content)) for d in by_ref.values()),
                                   **({"settings": wanted_settings} if wanted_settings is not None else {})}))
    cand = Candidate(refs, sorted(new_versions), auto, by_ref, digest, derived, wanted_settings)
    return [], cand


def release_id_for(h: str) -> str:
    return "rel-" + h[:16]


# --- app -----------------------------------------------------------------------------------------------------

Auth = Annotated[str | None, Header(alias="authorization")]


def problem(code: str, status: int, detail: str, trace_id: str, *, extra: dict[str, Any] | None = None,
            urn: str = "problem") -> Response:
    body: dict[str, Any] = {"type": f"urn:agentcore:{urn}:{code}", "title": PROBLEM_TITLES.get(code, code) if urn == "problem" else code,  # registry errors carry the code
                            "status": status, "code": code, "detail": detail, "trace_id": trace_id}
    body.update(extra or {})
    return Response(json.dumps(body), status_code=status, media_type="application/problem+json")


def create_app(limits: Limits | None = None) -> FastAPI:
    holder = {"st": State(limits or Limits())}
    faults: list[dict[str, Any]] = []
    app = FastAPI(title="registry-mock", version="1.0.0", docs_url=None, redoc_url=None, openapi_url="/openapi.json")

    def st() -> State:
        return holder["st"]

    # --- tracing + error handlers (same envelope as M9) ---
    @app.middleware("http")
    async def _trace(request: Request, call_next):  # type: ignore[no-untyped-def]
        request.state.trace_id = uuid.uuid4().hex
        return await call_next(request)

    def tid(request: Request) -> str:
        return str(getattr(request.state, "trace_id", "unknown"))

    @app.exception_handler(RegistryError)
    async def _reg(request: Request, exc: RegistryError) -> Response:
        extra: dict[str, Any] = {}
        if exc.code == "validation_failed":
            extra["violations"] = exc.payload
        elif exc.payload is not None:
            extra["payload"] = exc.payload
        return problem(exc.code, ERROR_STATUS[exc.code], exc.detail, tid(request), extra=extra, urn="registry")

    @app.exception_handler(CredentialsInvalid)
    async def _creds(request: Request, exc: CredentialsInvalid) -> Response:
        return problem("credentials_invalid", 401, "", tid(request))

    @app.exception_handler(PrincipalExpired)
    async def _expired(request: Request, exc: PrincipalExpired) -> Response:
        return problem("principal_expired", 401, "", tid(request))

    @app.exception_handler(RequestValidationError)
    async def _validation(request: Request, exc: RequestValidationError) -> Response:
        detail = "; ".join(f"{'.'.join(str(x) for x in e['loc'])}: {e['type']}" for e in exc.errors())
        return problem("invalid_request", 422, detail, tid(request))

    @app.exception_handler(StarletteHTTPException)
    async def _http(request: Request, exc: StarletteHTTPException) -> Response:
        code = "not_found" if exc.status_code == 404 else ("internal_error" if exc.status_code >= 500 else "invalid_request")
        return problem(code, exc.status_code, "", tid(request))

    @app.exception_handler(Exception)
    async def _unexpected(request: Request, exc: Exception) -> Response:
        return problem("internal_error", 500, "", tid(request))

    # --- auth ---
    def who(authorization: str | None) -> Principal:
        scheme, _, rest = (authorization or "").strip().partition(" ")
        credential = rest.strip() if scheme.lower() == "bearer" else (authorization or "").strip()
        if not credential:
            raise CredentialsInvalid()
        try:
            model = _PrincipalModel.model_validate(jws.verify(credential))
        except Exception:  # signature, header, kid, payload: all the same outwards
            raise CredentialsInvalid() from None
        if model.exp.astimezone(timezone.utc) <= st().clock.now():
            raise PrincipalExpired()
        p = Principal(model.type, model.id, model.roles, model.attrs, str(model.auth.get("level")), model.exp)
        if p.id is None or p.type != "builder":
            raise RegistryError("forbidden_role", "only a builder operates the registry")
        return p

    def need_constructor(p: Principal) -> None:
        if "constructor" not in p.roles:
            raise RegistryError("forbidden_role", "role constructor required")

    def need_human_step_up(p: Principal, role: str) -> None:
        if role not in p.roles or p.attrs.get("actor") != "human":
            raise RegistryError("forbidden_role", f"only a person with role {role} may do this")
        if p.auth_level != "step_up":
            raise RegistryError("step_up_required", "stronger authentication required")

    def proposal_json(p: ProposalRec) -> dict[str, Any]:
        return {"proposal_id": p.proposal_id, "agent_id": p.agent_id, "origin": p.origin, "state": p.state,
                "rev": p.rev, "base_release_id": p.base_release_id, "title": p.title, "created_by": p.created_by,
                "candidate_hash": p.candidate_hash, "updated_at": iso(p.updated_at)}

    def save(p: ProposalRec, **upd: Any) -> ProposalRec:
        for k, v in upd.items():
            setattr(p, k, v)
        p.updated_at = st().clock.now()
        return p

    def get_p(pid: str) -> ProposalRec:
        p = st().proposals.get(pid)
        if p is None:
            raise RegistryError("not_found", "proposal does not exist")
        return p

    def expect(p: ProposalRec, *states: str) -> None:
        if p.state not in states:
            raise RegistryError("illegal_transition", f"proposal is {p.state}; needs {', '.join(states)}")

    def j(value: Any, status: int = 200) -> Response:
        return Response(json.dumps(value, ensure_ascii=False), status_code=status, media_type="application/json")

    def release_json(r: Release) -> dict[str, Any]:
        base_refs = set(st().releases[r.base_release_id].refs) if r.base_release_id else set()
        has_base = r.base_release_id is not None
        return {"release_id": r.release_id, "status": r.status, "agent_id": r.agent_id,
                "entities": [{"ref": ref_json(ref), "content_hash": st().versions[ref].content_hash,
                              "docs": st().versions[ref].docs, "changed_vs_base": has_base and ref not in base_refs}
                             for ref in r.refs],
                "knowledge_snapshot": None, "proposal_id": r.proposal_id, "base_release_id": r.base_release_id,
                "published_by": r.published_by, "published_at": iso(r.published_at),
                "eval_suite_refs": [ref_json(x) for x in r.eval_suite_refs], **copy.deepcopy(r.settings)}

    def violations_for(items: list[dict[str, Any]]) -> list[dict[str, Any]]:
        return items

    def latest_eval(pid: str, h: str | None) -> dict[str, Any] | None:
        runs = [e for e in st().evals if e["proposal_id"] == pid and e["candidate_hash"] == h]
        return runs[-1] if runs else None

    router = APIRouter(prefix="/v1/registry")

    @router.post("/proposals", status_code=201)
    def create(request: Request, body: _Create, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_constructor(actor)
        s = st()
        if body.origin == "auto_detect":
            since = s.clock.now() - s.limits.window
            count = sum(1 for p in s.proposals.values() if p.origin == "auto_detect" and p.created_at > since)
            if count >= s.limits.proposals_per_day:
                raise RegistryError("quota_exceeded", f"{s.limits.proposals_per_day} autonomous proposals in 24h")
        bad = []
        if not ID_RE.fullmatch(body.agent_id):
            bad.append("agent_id")
        if not 1 <= len(body.title) <= 200:
            bad.append("title")
        if bad:
            raise RegistryError("validation_failed", "proposal is not valid",
                                [violation("REG-PROPOSAL", f"`{f}` is not valid", f) for f in sorted(bad)])
        now = s.clock.now()
        p = ProposalRec(s.new_id("proposal"), body.agent_id, body.origin, "draft", 0,
                        s.aliases.get((body.agent_id, "staging")), body.title, actor.id or "", None, now, now)
        s.proposals[p.proposal_id] = p
        return j(proposal_json(p), 201)

    def release_changes(s: State, p: ProposalRec, changes: list[EntityDraft]) -> list[dict[str, Any]]:
        """What `release_settings` changes against the base (N-07); without a base, against the defaults."""
        sets = settings_draft(changes)
        if not sets or settings_schema_problems(sets[0].content):
            return []
        base = s.releases.get(p.base_release_id) if p.base_release_id else None
        current: dict[str, Any] = {
            "interrupts": base.settings["interrupts"] if base else [],
            "language_detection": (base.settings["language_detection"] or {}).get("id") if base else None,
            "injection_ruleset": (base.settings["injection_ruleset"] or {}).get("id") if base and base.settings.get("injection_ruleset") else None,
            "max_input_chars": base.settings["max_input_chars"] if base else 4000}
        wanted = dict(sets[0].content)
        if wanted.get("interrupts") is not None:
            wanted["interrupts"] = [interrupt_norm(i) for i in wanted["interrupts"]]
        return [{"field": f, "before": current[f], "after": wanted[f]}
                for f in SETTINGS_FIELDS if wanted.get(f) is not None and wanted[f] != current[f]]

    @router.get("/proposals/{pid}")
    def show(request: Request, pid: str, authorization: Auth = None) -> Response:
        who(authorization)
        s = st()
        p = get_p(pid)
        last = latest_eval(pid, p.candidate_hash) if p.candidate_hash else None
        changes = s.changes.get(pid, [])
        review = None
        if last is not None:
            review = {"functional_changes": [c.model_dump() for c in changes if c.kind != "eval_suite"],
                      "release_changes": release_changes(s, p, changes),
                      "suite": last["suite"],
                      "suite_changes": [c.model_dump() for c in changes if c.kind == "eval_suite"],
                      "gate": last["report"]["items"], "yardstick_loosened": last["report"]["yardstick_changes"]}
        return j({"proposal": proposal_json(p), "changes": [c.model_dump() for c in changes], "last_eval": last,
                  "review": review})

    @router.put("/proposals/{pid}/draft")
    def draft(request: Request, pid: str, body: _Draft, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_constructor(actor)
        s = st()
        problems: list[dict[str, Any]] = []
        if len(body.changes) > s.limits.max_changes:
            problems = [violation("REG-LIMIT", f"proposal changes {len(body.changes)} entities; max {s.limits.max_changes}")]
        else:
            for d in body.changes:
                size = len(canonical(d.content))
                if size > s.limits.max_entity_bytes:
                    where = f"{d.kind[:40]}:{str(d.content.get('id'))[:80]}"
                    problems.append(violation("REG-LIMIT", f"{where} is {size} bytes; max {s.limits.max_entity_bytes}", where))
        if problems:
            raise RegistryError("validation_failed", "draft exceeds the limits", problems)
        if any(d.kind == SETTINGS_KIND and d.content.get("interrupts") is not None for d in body.changes) \
                and "admin" not in actor.roles:
            raise RegistryError("forbidden_role", "cambiar las interrupciones de la release exige el rol admin")
        p = get_p(pid)
        expect(p, "draft")
        if p.rev != body.expected_rev:
            raise RegistryError("proposal_stale", f"proposal is at revision {p.rev}, not {body.expected_rev}")
        s.changes[pid] = list(body.changes)
        save(p, rev=p.rev + 1)
        return j(proposal_json(p))

    @router.post("/proposals/{pid}/validate")
    def validate(request: Request, pid: str, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_constructor(actor)
        p = get_p(pid)
        problems, cand = build_candidate(st(), p)
        if cand is None:
            return j({"violations": problems, "candidate_hash": None, "auto_bumped": []})
        return j({"violations": [], "candidate_hash": cand.hash, "auto_bumped": [ref_json(r) for r in cand.auto_bumped]})

    @router.post("/proposals/{pid}/freeze")
    def freeze(request: Request, pid: str, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_constructor(actor)
        p = get_p(pid)
        expect(p, "draft")
        problems, cand = build_candidate(st(), p)
        if cand is None:
            raise RegistryError("validation_failed", f"candidate has {len(problems)} violations", problems)
        save(p, state="candidate", candidate_hash=cand.hash)
        return j({"proposal_id": pid, "candidate_hash": cand.hash, "release_id_preview": release_id_for(cand.hash),
                  "new_versions": [ref_json(r) for r in cand.new_versions],
                  "auto_bumped": [ref_json(r) for r in cand.auto_bumped]})

    @router.post("/proposals/{pid}/reopen")
    def reopen(request: Request, pid: str, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_constructor(actor)
        p = get_p(pid)
        expect(p, "candidate", "evaluated", "approved")
        save(p, state="draft", candidate_hash=None, rev=p.rev + 1)
        return j(proposal_json(p))

    @router.post("/proposals/{pid}/evaluate")
    def evaluate(request: Request, pid: str, body: _Evaluate, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_constructor(actor)
        s = st()
        p = get_p(pid)
        expect(p, "candidate")
        evals = sum(1 for e in s.evals if e["proposal_id"] == pid) if p.origin == "auto_detect" else 0
        if evals >= s.limits.evals_per_proposal:
            raise RegistryError("quota_exceeded", f"proposal already has {s.limits.evals_per_proposal} evaluations")
        problems, cand = build_candidate(s, p)
        if cand is None or cand.hash != p.candidate_hash:
            raise RegistryError("candidate_changed", "candidate changed since freeze",
                                problems if cand is None else None)
        suite_ref = next((r for r in cand.drafts if r[0] == "eval_suite" and r[1] == body.suite_id
                          and body.suite_version in (None, r[2])), None)
        if suite_ref is None:
            stored = sorted(r for r in s.versions if r[0] == "eval_suite" and r[1] == body.suite_id
                            and body.suite_version in (None, r[2]))
            if not stored:
                raise RegistryError("not_found", f"suite {body.suite_id} does not exist")
            suite_ref = stored[-1]
        verdict = s.eval_script.pop(0) if s.eval_script else "pass"
        detail = "contract_fixture" + (": timeout" if verdict == "timeout" else "")
        verdict = "failed_infra" if verdict == "timeout" else verdict
        report = {"verdict": verdict, "items": [], "runs": None, "results": [], "judge_notes": [],
                  "yardstick_changes": [], "detail": detail}
        run = {"eval_run_id": s.new_id("eval_run"), "proposal_id": pid, "candidate_hash": cand.hash,
               "base_release_id": p.base_release_id, "suite": ref_json(suite_ref), "verdict": verdict,
               "report": report, "at": iso(s.clock.now())}
        s.evals.append(run)
        if verdict == "pass":
            save(p, state="evaluated")
        elif verdict == "fail":
            save(p, state="draft", candidate_hash=None, rev=p.rev + 1)
            # N-10: the failing EvalRun id travels in the 409 body (the run itself has no read route upstream)
            raise RegistryError("gate_failed", "candidate does not pass the gate", {**report, "eval_run_id": run["eval_run_id"]})
        return j(report)

    @router.post("/proposals/{pid}/approve")
    def approve(request: Request, pid: str, body: _Approve, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_human_step_up(actor, "aprobador")
        s = st()
        p = get_p(pid)
        expect(p, "evaluated")
        if body.candidate_hash != p.candidate_hash:
            raise RegistryError("candidate_changed", "approved candidate is not the current one")
        run = latest_eval(pid, body.candidate_hash)
        if run is None or run["verdict"] != "pass":
            raise RegistryError("gate_failed", "no passing evaluation")
        approval = {"proposal_id": pid, "candidate_hash": body.candidate_hash, "actor": actor.id, "decision": "approved",
                    "reason": None, "yardstick_loosened": [], "at": iso(s.clock.now())}
        s.approvals.append(approval)
        save(p, state="approved")
        return j(approval)

    @router.post("/proposals/{pid}/reject")
    def reject(request: Request, pid: str, body: _Reason, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_human_step_up(actor, "aprobador")
        s = st()
        p = get_p(pid)
        expect(p, "evaluated")
        s.approvals.append({"proposal_id": pid, "candidate_hash": p.candidate_hash or "", "actor": actor.id,
                            "decision": "rejected", "reason": body.reason[:2000], "yardstick_loosened": [],
                            "at": iso(s.clock.now())})
        save(p, state="draft", candidate_hash=None, rev=p.rev + 1)
        return j(proposal_json(p))

    @router.post("/proposals/{pid}/publish")
    def publish(request: Request, pid: str, idempotency_key: Annotated[str, Header(max_length=255)],
                authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_human_step_up(actor, "aprobador")
        s = st()
        prior = s.publish_keys.get(idempotency_key)
        if prior is not None:
            if prior[0] != pid:
                raise RegistryError("illegal_transition", "Idempotency-Key already used with another proposal")
            return j(release_json(s.releases[prior[1]]))
        p = get_p(pid)
        expect(p, "approved")
        current = s.aliases.get((p.agent_id, "staging"))
        if current != p.base_release_id:
            save(p, state="draft", candidate_hash=None, base_release_id=current, rev=p.rev + 1)
            raise RegistryError("proposal_stale", "staging moved since the proposal was created; freeze and evaluate again")
        problems, cand = build_candidate(s, p)
        if cand is None or cand.hash != p.candidate_hash:
            raise RegistryError("candidate_changed", "candidate changed since approval", problems or None)
        approval = next((a for a in reversed(s.approvals) if a["proposal_id"] == pid and a["candidate_hash"] == cand.hash), None)
        run = latest_eval(pid, cand.hash)
        if approval is None or approval["decision"] != "approved":
            raise RegistryError("gate_failed", "missing current approval")
        if run is None or run["verdict"] != "pass":
            raise RegistryError("gate_failed", "no passing evaluation")
        rid = release_id_for(cand.hash)
        if rid in s.releases:
            raise RegistryError("illegal_transition", "that release already exists")
        now, who_id = s.clock.now(), actor.id or ""
        for ref in cand.new_versions:
            if ref in cand.drafts:
                d = cand.drafts[ref]
                s.versions[ref] = Version(ref, d.content, sha256_hex(canonical(d.content)), d.docs.model_dump(), p.created_by, now)
            else:  # derived agent version
                assert cand.derived_agent is not None
                s.versions[ref] = Version(
                    ref, cand.derived_agent, sha256_hex(canonical(cand.derived_agent)),
                    {"description": f"Version derivada de {p.agent_id}",
                     "rationale": "Actualiza referencias a versiones nuevas de la propuesta",
                     "changelog": ", ".join(f"{r[1]}@{r[2]}" for r in cand.new_versions if r[0] != "agent")}, p.created_by, now)
        refs = sorted((r for k, r in cand.refs.items() if k[0] != "eval_suite"), key=ref_str)
        suite_ref = (run["suite"]["kind"], run["suite"]["id"], run["suite"]["version"])
        base_settings = copy.deepcopy(s.releases[p.base_release_id].settings) if p.base_release_id in s.releases else {}
        for key, value in (cand.settings or {}).items():
            if key == "interrupts":
                base_settings[key] = [interrupt_norm(i) for i in value]
            elif key in ("language_detection", "injection_ruleset"):
                version = next((r[2] for k, r in cand.refs.items() if k == (key, value)), None)
                base_settings[key] = {"id": value, "spec": version} if version else base_settings.get(key)
            else:
                base_settings[key] = value
        s.releases[rid] = Release(rid, p.agent_id, refs, p.base_release_id, pid, who_id, now, [suite_ref],
                                  settings=copy.deepcopy(base_settings))
        s.aliases[(p.agent_id, "staging")] = rid
        save(p, state="published")
        s.publish_keys[idempotency_key] = (pid, rid)
        return j(release_json(s.releases[rid]))

    @router.post("/aliases/{agent_id}/{alias}")
    def promote(request: Request, agent_id: str, alias: str, body: _Promote, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_human_step_up(actor, "aprobador")
        s = st()
        if alias not in ("prod", "staging"):
            raise RegistryError("validation_failed", "alias must be one of: prod, staging")
        r = s.releases.get(body.release_id)
        if r is None:
            raise RegistryError("not_found", "release does not exist")
        if r.status != "active":
            raise RegistryError("illegal_transition", "a revoked release is not promoted")
        if r.agent_id != agent_id:
            raise RegistryError("illegal_transition", "the release does not contain the agent")
        change = {"agent_id": agent_id, "alias": alias, "before": s.aliases.get((agent_id, alias)),
                  "after": body.release_id, "actor": actor.id, "reason": body.reason[:500], "at": iso(s.clock.now())}
        s.aliases[(agent_id, alias)] = body.release_id
        return j(change)

    @router.post("/releases/{rid}/revoke")
    def revoke(request: Request, rid: str, body: _Reason, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_human_step_up(actor, "admin")
        s = st()
        r = s.releases.get(rid)
        if r is None:
            raise RegistryError("not_found", "release does not exist")
        if r.status != "active":
            raise RegistryError("illegal_transition", "release is already revoked")
        if any(a == "prod" and target == rid for (_, a), target in s.aliases.items()):
            raise RegistryError("illegal_transition", "prod points at this release: promote another first")
        r.status = "revoked"
        return j(release_json(r))

    @router.get("/releases/{rid}")
    def release(request: Request, rid: str, authorization: Auth = None) -> Response:
        who(authorization)
        r = st().releases.get(rid)
        if r is None:
            raise RegistryError("not_found", "release does not exist")
        return j(release_json(r))

    @router.get("/releases/{a}/diff/{b}")
    def diff(request: Request, a: str, b: str, authorization: Auth = None) -> Response:
        who(authorization)
        s = st()
        for rid in (a, b):
            if rid not in s.releases:
                raise RegistryError("not_found", "release does not exist")
        left = {(r[0], r[1]): r for r in s.releases[a].refs}
        right = {(r[0], r[1]): r for r in s.releases[b].refs}
        changed = [{"before": ref_json(left[k]), "after": ref_json(right[k]), "docs": s.versions[right[k]].docs}
                   for k in sorted(left.keys() & right.keys()) if left[k] != right[k]]
        return j({"a": a, "b": b, "added": [ref_json(right[k]) for k in sorted(right.keys() - left.keys())],
                  "removed": [ref_json(left[k]) for k in sorted(left.keys() - right.keys())], "changed": changed})

    @router.get("/entities/{kind}/{eid:path}")
    def entity(request: Request, kind: str, eid: str, authorization: Auth = None) -> Response:
        who(authorization)
        version = request.query_params.get("version")
        found = sorted((v for r, v in st().versions.items() if r[0] == kind and r[1] == eid and version in (None, r[2])),
                       key=lambda v: semver(v.ref[2]) or (0, 0, 0))
        if not found:
            raise RegistryError("not_found", "entity or version does not exist")
        v = found[-1]
        return j({"ref": ref_json(v.ref), "content": v.content, "content_hash": v.content_hash, "docs": v.docs,
                  "created_by": v.created_by, "created_at": iso(v.created_at)})

    @router.get("/aliases/{agent_id}/{alias}")
    def read_alias(request: Request, agent_id: str, alias: str, authorization: Auth = None) -> Response:
        who(authorization)  # same permission as the other reads: any builder, no proposal, no quota (N-02)
        s = st()
        release_id = s.aliases.get((agent_id, alias))
        if release_id is None or release_id not in s.releases:
            raise RegistryError("not_found", "the alias does not point to any release")
        return j({"agent_id": agent_id, "alias": alias, "release_id": release_id,
                  "status": s.releases[release_id].status})

    @router.get("/versions/{kind}/{eid:path}")
    def read_versions(request: Request, kind: str, eid: str, authorization: Auth = None) -> Response:
        who(authorization)
        found = sorted((v for r, v in st().versions.items() if r[0] == kind and r[1] == eid),
                       key=lambda v: semver(v.ref[2]) or (0, 0, 0))
        return j([{"ref": ref_json(v.ref), "content_hash": v.content_hash, "docs": v.docs,
                   "created_by": v.created_by, "created_at": iso(v.created_at)} for v in found])  # unknown id/kind: []

    @router.get("/runs/{run_id}/lineage")
    def lineage(request: Request, run_id: str, authorization: Auth = None) -> Response:
        actor = who(authorization)
        need_constructor(actor)
        raise RegistryError("not_found", "run does not exist")  # the mock has no runs: same as a2 without a run reader

    app.include_router(router)

    # --- /_sim/* control channel (absent from the real server; faults are OFF by default) ---
    @app.get("/_sim/info")
    def info() -> dict[str, str]:
        return {"pinned_sha": PIN_SHA, "contract_version": CONTRACT_VERSION, "fixtures_digest": fixtures_digest(),
                "level": "mock"}

    @app.post("/_sim/reset")
    def reset() -> dict[str, str]:
        holder["st"] = State(holder["st"].limits)
        faults.clear()
        return {"status": "reset"}

    @app.post("/_sim/clock/advance")
    async def advance(request: Request) -> JSONResponse:
        st().clock.advance(float((await request.json())["seconds"]))
        return JSONResponse({"now": iso(st().clock.now())})

    @app.post("/_sim/eval")
    async def program_eval(request: Request) -> dict[str, list[str]]:
        script = list((await request.json())["script"])
        if any(x not in ("pass", "fail", "failed_infra", "timeout") for x in script):
            raise RegistryError("validation_failed", "unknown eval verdict")
        st().eval_script = script
        return {"script": script}

    @app.post("/_sim/fault")
    async def program_fault(request: Request) -> dict[str, Any]:
        spec = await request.json()
        if spec.get("mode") not in ("status500", "latency", "drop_after_commit", "disconnect"):
            raise RegistryError("validation_failed", "unknown fault mode")
        faults.append(spec)
        return {"queued": len(faults)}

    inner = app

    async def asgi(scope: dict[str, Any], receive: Any, send: Any) -> None:
        """Fault layer: only for `/v1/registry/*`, only when armed through `/_sim/fault`."""
        if scope["type"] != "http" or not scope["path"].startswith("/v1/registry") or not faults:
            await inner(scope, receive, send)
            return
        spec = faults.pop(0)
        mode = spec["mode"]
        if mode == "latency":
            await asyncio.sleep(float(spec.get("seconds", 1)))
            await inner(scope, receive, send)
            return

        async def broken() -> None:
            await send({"type": "http.response.start", "status": 200,
                        "headers": [(b"content-type", b"application/json"), (b"content-length", b"100")]})
            await send({"type": "http.response.body", "body": b"{", "more_body": False})

        if mode == "status500":
            await send({"type": "http.response.start", "status": 500, "headers": [(b"content-type", b"application/problem+json")]})
            body = json.dumps({"type": "urn:agentcore:problem:internal_error", "title": "Error interno", "status": 500,
                               "code": "internal_error", "detail": "", "trace_id": uuid.uuid4().hex}).encode()
            await send({"type": "http.response.body", "body": body})
            return
        if mode == "disconnect":
            await broken()
            return

        async def swallow(_message: dict[str, Any]) -> None:
            return None

        await inner(scope, receive, swallow)  # state committed, response lost
        await broken()

    return _Wrapper(app, asgi)  # type: ignore[return-value]


class _Wrapper:
    """ASGI callable that exposes the FastAPI attributes tests use (`openapi`, `routes`)."""

    def __init__(self, app: FastAPI, asgi: Any) -> None:
        self._app, self._asgi = app, asgi

    async def __call__(self, scope: dict[str, Any], receive: Any, send: Any) -> None:
        await self._asgi(scope, receive, send)

    def __getattr__(self, name: str) -> Any:
        return getattr(self._app, name)
