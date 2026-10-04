"""INT0: real-Core `CoreHooks` for the E2E-THREAD-01 runner, over the existing bridge client (`codex_standin.bridge.Bridge`).

Real here: `dry_run` (step 5, `POST /core-authoring/dry-run`), `alias_read` (`GET /core-state/aliases/...`) and, through
`RealCore`, the frozen proposal of the thread's own draft, `run_arms`, `approve`
and `publish`. No secrets are handled here: the bridge client owns its signing key.
"""
from __future__ import annotations

import re
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

import yaml

from codex_standin.dto import arm_request, request_digest
from codex_standin.engine import put_draft_digest

from . import compile_step as cmp

_HEX64 = re.compile(r"^[0-9a-f]{64}$")
_REF = re.compile(r"^([a-z_]+):([A-Za-z0-9._-]+)@([0-9]+)$")
DOCS = {"description": "thread01 change", "rationale": "recorded synthetic opportunity", "changelog": "thread01"}


def _load(path: Path) -> dict:
    return yaml.safe_load(path.read_text("utf-8"))


def changes_from_ops(world: dict, ops: list[dict]) -> list[dict]:
    """Compiled operations -> Core `ChangeIn` drafts: the base asset content with the version of `new_ref`."""
    slots, out = cmp._slots(world), []
    for op in ops:
        kind = op["target_kind"]
        content = _load(slots[kind]["file"])
        content["version"] = f"{_REF.match(op['new_ref']).group(3)}.0.0"
        out.append({"kind": kind, "content": content, "docs": dict(DOCS)})
    return out


def make_dry_run(bridge: Any, world: dict, *, tenant: str, agent_id: str, base_release_id: str | None) -> Callable:
    def dry_run(ops: list[dict]) -> str:
        body = {"schema_version": "1", "tenant_id": tenant, "agent_id": agent_id, "base_release_id": base_release_id,
                "changes": changes_from_ops(world, ops)}
        r = bridge.call("POST", "/core-authoring/dry-run", "dry_run", tenant, json=body)
        if r.status_code != 200:
            raise RuntimeError(f"core dry-run http {r.status_code}")
        res = r.json()
        if not res.get("valid"):
            raise RuntimeError("core dry-run refused: " + "; ".join(f"{v['rule']}: {v['message']}" for v in res["violations"]))
        if res.get("request_digest") != request_digest(body):  # the answer must be about the request we sent
            raise RuntimeError("core dry-run request_digest does not match the request sent")
        if res.get("proposal_created") is not False:
            raise RuntimeError("core dry-run reported proposal_created != false (dry-run must write nothing)")
        h = res.get("candidate_hash")
        h = h[7:] if isinstance(h, str) and h.startswith("sha256:") else h
        if not isinstance(h, str) or not _HEX64.match(h):
            raise RuntimeError("core dry-run valid answer without a well-formed candidate_hash")
        return "sha256:" + h
    return dry_run


def make_alias_read(bridge: Any, *, tenant: str, agent_id: str) -> Callable:
    def alias_read(ctx: Any, alias: str) -> dict:
        r = bridge.call("GET", f"/core-state/aliases/{agent_id}/{alias}", "aliases", tenant)
        if r.status_code != 200:
            raise RuntimeError(f"core alias read http {r.status_code}")
        res = r.json()
        if res.get("alias") != alias or not isinstance(res.get("release_id"), str) or not res["release_id"]:
            raise RuntimeError("core alias read answer does not match the requested alias or lacks release_id")
        return {"release_id": res["release_id"], "alias": alias}
    return alias_read


OPS = ["create_proposal", "put_draft", "freeze"]
SUPERVISOR = "local-supervisor"
ORACLE = "oracle:suite-expect@handwritten"  # the suite's own `expect` blocks (authored by the suite author, not by Claude)
WRITER = "pulso-writer"


@dataclass
class Frozen:
    proposal_id: str
    candidate_hash: str
    base_release_id: str
    changes: list
    binding_ref: str
    title: str
    plan_ref: str


def _drafts_digest(changes: list[dict]) -> str:
    from agent_core.registry.models import EntityDraft
    from codex_standin.dto import digest_json
    drafts = sorted((EntityDraft.model_validate(c) for c in changes), key=lambda d: (d.kind, str(d.content.get("id", ""))))
    return digest_json([d.model_dump(mode="json") for d in drafts])


class _Timed:
    """Records seconds per live call (the INT0 'seconds per call' log) without changing the call."""

    def __init__(self, inner: Any, timings: list) -> None:
        self._inner, self._t = inner, timings

    def call(self, method: str, path: str, op: str, tenant: Any, **kw: Any) -> Any:
        return self._time(f"{op} {method} {path.split('?')[0]}", lambda: self._inner.call(method, path, op, tenant, **kw))

    def arm_run(self, tenant: str, body: dict) -> Any:
        return self._time(f"arm_run {body['arm']} {body['case_ref']}", lambda: self._inner.arm_run(tenant, body))

    def _time(self, name: str, fn: Callable) -> Any:
        t0 = time.monotonic()
        try:
            return fn()
        finally:
            self._t.append({"call": name, "seconds": round(time.monotonic() - t0, 3)})


class RealCore:
    """Real-Core hooks for steps 5-9 of the thread. `engine` is the codex_standin Engine (writer stage via the bridge),
    `registry(method, path, bearer, **kw)` calls Core's `/v1/registry`, `authorize(operation, target) -> jws` obtains a
    human-issuer command-authorization JWS (the local issuer, see `make_human_authorizer`)."""

    def __init__(self, *, engine: Any, bridge: Any, registry: Callable, authorize: Callable, world: dict, tenant: str,
                 agent_id: str, arm_profile: str | None = None) -> None:
        self.engine, self.registry, self.authorize = engine, registry, authorize
        self.world, self.tenant, self.agent_id, self.arm_profile = world, tenant, agent_id, arm_profile
        self.timings: list[dict] = []
        self.bridge = _Timed(bridge, self.timings)
        self._base: str | None = None
        self._frozen: Frozen | None = None
        self._approved = False
        self._evaluation: dict | None = None
        self._published: str | None = None
        self._bot: str | None = None

    # -- helpers ---------------------------------------------------------------------------------------------------
    def base_release(self) -> str:
        if self._base is None:
            self._base = make_alias_read(self.bridge, tenant=self.tenant, agent_id=self.agent_id)(None, "prod")["release_id"]
        return self._base

    def _reg(self, name: str, method: str, path: str, bearer: str, **kw: Any) -> Any:
        t0 = time.monotonic()
        try:
            return self.registry(method, path, bearer, **kw)
        finally:
            self.timings.append({"call": f"{name} {method} {path.split('/')[1]}", "seconds": round(time.monotonic() - t0, 3)})

    def _bot_token(self) -> str:
        if self._bot is None:
            r = self.bridge.call("POST", "/core-credentials/issue", "credentials", self.tenant, json={
                "tenant_id": self.tenant, "role": "constructor", "purpose": "registry_write"})
            if r.status_code != 200:
                raise RuntimeError(f"core credential issue http {r.status_code}")
            self._bot = r.json()["jws"]
        return self._bot

    def _rev(self, fz: Frozen) -> int:
        r = self._reg("proposal_read", "GET", f"/proposals/{fz.proposal_id}", self._bot_token())
        if r.status_code != 200:
            raise RuntimeError(f"core proposal read http {r.status_code}")
        return int(r.json()["proposal"]["rev"])

    @staticmethod
    def _target(fz: Frozen, rev: int, hash_: str | None = None) -> dict:
        return {"proposal_id": fz.proposal_id, "candidate_hash": hash_ or fz.candidate_hash, "expected_revision": rev}

    # -- step 5 ----------------------------------------------------------------------------------------------------
    def dry_run(self, ops: list[dict]) -> str:
        return make_dry_run(self.bridge, self.world, tenant=self.tenant, agent_id=self.agent_id,
                            base_release_id=self.base_release())(ops)

    # -- frozen proposal of the thread's draft ---------------------------------------------------------------------
    def freeze(self, ctx: Any) -> Frozen:
        if self._frozen is not None:
            return self._frozen
        plan = ctx.out["compiled"]["draft_plan"]
        digest = plan["digest"].split(":", 1)[1]
        changes = changes_from_ops(ctx.out.get("world") or self.world, plan["operations"])
        base, n = self.base_release(), str(time.time_ns())[-12:]
        title, plan_id = f"pulso-key:thread01-{n}", f"plan-t01-{n}"
        self.engine.seal(plan_id, {"agent_id": self.agent_id, "title": title, "changes": changes})
        commitment = {"mode": "write", "base_release_id": base, "create_agent_id": self.agent_id,
                      "create_origin": "builder_chat", "create_title": title,
                      "put_draft_digest": put_draft_digest(None, None, changes), "operations": OPS}
        t0 = time.monotonic()
        st = self.engine.stage("writer", f"job-writer-t01-{n}", "writer", WRITER, {
            "draft_plan_ref": plan_id, "proposal_id": None, "base_release_id": base, "evaluate_enabled": False},
            registry_mutation_commitment=commitment)
        self.timings.append({"call": "writer_stage invoke", "seconds": round(time.monotonic() - t0, 3)})
        if st.response.status_code != 200 or not st.out.get("core_run_id"):
            raise RuntimeError(f"core writer stage http {st.response.status_code}")
        wr = (self.engine.facts(st.out["core_run_id"]).get("pulso_writer_receipts") or {}).get("value") or {}
        if [r.get("op") for r in wr.get("write_receipts", [])] != OPS or not all(r.get("verified") for r in wr["write_receipts"]):
            raise RuntimeError("core writer did not commit create_proposal, put_draft, freeze")
        if wr.get("candidate_hash") != digest:
            raise RuntimeError(f"frozen candidate_hash {wr.get('candidate_hash')} differs from the draft digest {digest}")
        self._frozen = Frozen(wr["proposal_id"], digest, base, changes, st.out["task_binding_ref"], title, plan_id)
        return self._frozen

    # -- step 6 ----------------------------------------------------------------------------------------------------
    def run_arms(self, ctx: Any) -> dict:
        fz = self.freeze(ctx)
        rev = self._rev(fz)
        scenarios = next(c for c in fz.changes if c["kind"] == "eval_suite")["content"]["scenarios"]
        target_c = {"kind": "frozen_candidate", "proposal_id": fz.proposal_id, "expected_rev": rev,
                    "base_release_id": fz.base_release_id, "candidate_hash": fz.candidate_hash,
                    "draft_plan_digest": _drafts_digest(fz.changes)}
        target_b = {"kind": "published_release", "release_id": fz.base_release_id}
        n = str(time.time_ns())[-10:]
        out: dict[str, list] = {"base": [], "candidate": []}
        for sc in scenarios:  # one arm run per scenario: the ArmReport is per run, the gate compares per case
            mid = self.engine.seal(f"manifest-t01-{n}-{sc['id']}", {"scenarios": [sc], "entries": {sc["id"]: {}}})
            for arm, target, side in (("baseline", target_b, "base"), ("candidate", target_c, "candidate")):
                body = arm_request(key=f"t01-{n}-{sc['id']}-{arm}", binding_ref=fz.binding_ref, manifest_ref=mid, profile=self.arm_profile,
                                   target=target, arm=arm, case_ref=sc["id"])
                r = self.bridge.arm_run(self.tenant, body)
                if r.status_code != 200:
                    raise RuntimeError(f"core arm {arm}/{sc['id']} http {r.status_code}")
                out[side].append(_run_of(r.json(), arm, sc["id"]))
        return out

    # -- Core's native evaluation (the registry requires `evaluated` before approval) --------------------------------
    def evaluate(self, ctx: Any) -> dict:
        """One native evaluation per thread (a second one would be a second `evaluated` transition)."""
        if self._evaluation is None:
            self._evaluation = self._evaluate(ctx)
        return self._evaluation

    def _evaluate(self, ctx: Any) -> dict:
        import hashlib

        from agent_core.registry import EvalSuite
        from agent_core.registry.entities import content_hash
        from codex_standin.dto import admission, evaluation_context_ref, idempotency_key
        fz = self.freeze(ctx)
        suite = next(c for c in fz.changes if c["kind"] == "eval_suite")["content"]
        n = str(time.time_ns())[-12:]
        job = f"job-evalonly-t01-{n}"
        key = idempotency_key(self.tenant, job, "writer", 1, "evalonly")
        bref = hashlib.sha256(f"{self.tenant}|{key}".encode()).hexdigest()
        self.engine.configure(preauthorized_bindings=[{"tenant": self.tenant, "binding_ref": bref}])
        body = admission(binding_ref=bref, proposal_id=fz.proposal_id, candidate_hash=fz.candidate_hash, suite_id=suite["id"],
                         suite_version=suite["version"], suite_digest=str(content_hash(EvalSuite.model_validate(suite))),
                         budget_ref="bud-e2e")
        adm = self._timed("admit", lambda: self.engine.bridge.admit(self.tenant, job, body))
        if adm.status_code not in (200, 201):
            raise RuntimeError(f"core evaluation admission http {adm.status_code} {_code(adm)}")
        ctx_ref = evaluation_context_ref(self.tenant, job, bref, fz.proposal_id, fz.candidate_hash, 1)
        st = self._timed("evaluate_only invoke", lambda: self.engine.stage(
            "writer", job, "evalonly", WRITER, {"draft_plan_ref": fz.plan_ref, "proposal_id": fz.proposal_id,
                                               "base_release_id": fz.base_release_id, "evaluate_enabled": True,
                                               "evaluation_suite_id": suite["id"], "evaluation_suite_version": suite["version"]},
            registry_mutation_commitment={"mode": "evaluate_only", "proposal_id": fz.proposal_id,
                                          "base_release_id": fz.base_release_id, "evaluate_enabled": True,
                                          "evaluation_context_ref": ctx_ref, "operations": []}))
        if st.response.status_code != 200 or not st.out.get("core_run_id"):
            raise RuntimeError(f"core evaluate-only stage http {st.response.status_code} {_code(st.response)}")
        wr = (self.engine.facts(st.out["core_run_id"]).get("pulso_writer_receipts") or {}).get("value") or {}
        return wr.get("native_evaluation") or {}

    def gate_probe(self, ctx: Any) -> dict:
        """What the real Core does with the thread's frozen draft when the human acts on it: used to document a blocked
        step 8/9 (the registry needs `evaluated` first). Moves nothing when blocked; reports the exact refusals."""
        fz = self.freeze(ctx)
        before = self.alias_read_raw("staging")
        ev = self.evaluate(ctx)
        a = self._reg("approve", "POST", f"/proposals/{fz.proposal_id}/approve",
                      self.authorize("approve", self._target(fz, self._rev(fz))), json={"candidate_hash": fz.candidate_hash})
        p = self._reg("publish", "POST", f"/proposals/{fz.proposal_id}/publish",
                      self.authorize("publish", self._target(fz, self._rev(fz))),
                      headers={"Idempotency-Key": "pub-" + uuid.uuid4().hex})
        return {"evaluation": ev.get("verdict"), "approve": [a.status_code, _code_of(a)], "publish": [p.status_code, _code_of(p)],
                "staging_unchanged": self.alias_read_raw("staging") == before}

    def alias_read_raw(self, alias: str) -> str:
        return make_alias_read(self.bridge, tenant=self.tenant, agent_id=self.agent_id)(None, alias)["release_id"]

    def _timed(self, name: str, fn: Callable) -> Any:
        t0 = time.monotonic()
        try:
            return fn()
        finally:
            self.timings.append({"call": name, "seconds": round(time.monotonic() - t0, 3)})

    # -- step 8 ----------------------------------------------------------------------------------------------------
    def approve(self, ctx: Any) -> dict:
        fz = self.freeze(ctx)
        ev = self.evaluate(ctx)  # the registry approves only an `evaluated` proposal (approve-before-evaluation is illegal)
        if ev.get("verdict") != "pass":
            raise RuntimeError(f"core native evaluation did not pass ({ev.get('verdict')}): the registry cannot approve")
        bad = "0" * 64  # a JWS authorised for a hash that is not the candidate must be refused by Core
        r = self._reg("approve_wrong_hash", "POST", f"/proposals/{fz.proposal_id}/approve",
                      self.authorize("approve", self._target(fz, self._rev(fz), bad)), json={"candidate_hash": bad})
        tamper = r.status_code == 409 and r.json().get("code") == "candidate_changed"
        jws = self.authorize("approve", self._target(fz, self._rev(fz)))
        r = self._reg("approve", "POST", f"/proposals/{fz.proposal_id}/approve", jws, json={"candidate_hash": fz.candidate_hash})
        if r.status_code != 200:
            raise RuntimeError(f"core approve http {r.status_code} {_code(r)}")
        body = r.json()
        again = self._reg("approve_replay", "POST", f"/proposals/{fz.proposal_id}/approve", jws,
                          json={"candidate_hash": fz.candidate_hash})
        replay = again.status_code == 409 and again.json().get("code") == "illegal_transition"
        self._approved = body.get("decision") == "approved"
        return {"approver": body.get("actor"), "decision": body.get("decision"), "candidate_hash": body.get("candidate_hash"),
                "tamper_refused": tamper, "replay_refused": replay}

    # -- step 9 ----------------------------------------------------------------------------------------------------
    def publish(self, ctx: Any) -> dict:
        if self._frozen is None or not self._approved:
            raise RuntimeError("proposal not approved by Core: nothing to publish")
        fz = self._frozen
        jws = self.authorize("publish", self._target(fz, self._rev(fz)))
        r = self._reg("publish", "POST", f"/proposals/{fz.proposal_id}/publish", jws,
                      headers={"Idempotency-Key": "pub-" + uuid.uuid4().hex})
        if r.status_code != 200 or not r.json().get("release_id"):
            raise RuntimeError(f"core publish http {r.status_code} {_code(r)}")
        self._published = r.json()["release_id"]
        return {"release_id": self._published, "alias": "staging"}

    def alias_read(self, ctx: Any, alias: str) -> dict:
        if alias == "staging" and self._published is None:
            raise RuntimeError("alias read before publish: staging cannot show the thread's release yet")
        return make_alias_read(self.bridge, tenant=self.tenant, agent_id=self.agent_id)(ctx, alias)

    def hooks(self, *, arms: bool = True, blocked: dict | None = None) -> Any:
        """`arms=False` leaves step 6 on the stand-in (e.g. blocked(jev): the Core has no `jev` decision provider)."""
        from . import thread01 as T
        return T.CoreHooks(
            dry_run=self.dry_run, run_arms=self.run_arms if arms else None, approve=self.approve, publish=self.publish,
            alias_read=self.alias_read, blocked=dict(blocked or {}),
            doubles=[{"port": "control_api", "provenance": "e2e-fixtures-double (binding, authz, broker, lab, bank)"}])


def _run_of(rep: dict, arm: str, case: str) -> dict:
    st = rep.get("status")
    if st == "completed":
        status = "completed"
    elif st == "candidate_failed":
        status = "failed"
    else:  # failed_infra / unknown: no evidence either way
        raise RuntimeError(f"core arm {arm}/{case} ended {st}: not evidence ({rep.get('reason')})")
    return {"arm": arm, "case_ref": case, "status": status, "closed_early": bool(rep.get("closed_early")),
            "cost_known": rep.get("cost_known") is True, "oracle_ref": rep.get("oracle_ref") or ORACLE,
            "final_state_ref": rep.get("final_state_ref"), "effect_receipts": rep.get("effect_receipts") or [],
            "reason": rep.get("reason")}


def make_registry(runtime_url: str, timeout: float = 60.0) -> Callable:
    """`/v1/registry` caller (httpx, bearer = the exact JWS bytes)."""
    import httpx

    def call(method: str, path: str, bearer: str, **kw: Any) -> Any:
        headers = {"Authorization": f"Bearer {bearer}", **kw.pop("headers", {})}
        return httpx.request(method, f"{runtime_url}/v1/registry{path}", headers=headers, timeout=timeout, **kw)
    return call


def make_human_authorizer(port: Any, *, tenant: str, actor: str = SUPERVISOR) -> Callable:
    """Durable single-use intention -> command-authorization JWS from the local human issuer (HumanAuthorizationPort)."""
    def authorize(operation: str, target: dict) -> str:
        it = port.create_intention(tenant_id=tenant, actor_ref=actor, operation=operation, target=target)
        return port.authorize(it.intention_id).reveal()
    return authorize


def _code(r: Any) -> str:
    try:
        b = r.json()
        return f"{b.get('code') or b.get('state')}: {str(b.get('message') or b.get('detail') or b.get('reason') or '')[:160]} {b.get('outcome') or ''}"
    except Exception:  # noqa: BLE001 - diagnostics only
        return ""


def _code_of(r: Any) -> str | None:
    try:
        return r.json().get("code")
    except Exception:  # noqa: BLE001
        return None
