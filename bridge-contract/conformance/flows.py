"""Golden flows: ONE definition used both to RECORD `examples/` (CONTRACT_RECORD=1, real runtime) and to REPLAY/CHECK
them against any target. Each step sends a real request, validates the response against the published schema, and
compares the NORMALISED exchange with the checked-in golden file.

Normalisation (stable ids, redaction): ids that differ per run (core_run_id, proposal_id, candidate_hash, eval_run_ref,
report_digest, deadline, exp ...) become `<key#n>` placeholders numbered by first appearance within a flow; service JWTs
and principal JWS are never stored (the request records the JWT *claims profile*); strings longer than 200 chars are
elided as `<string len=N sha256=XXXXXXXX>`."""

from __future__ import annotations

import hashlib
import json
import re
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import pytest

from conformance.conftest import assert_valid
from conformance.kit import evaluation_context_ref, idempotency_key
from conformance.worlds import World

EXAMPLES = Path(__file__).resolve().parents[1] / "examples"
TRACEPARENT = "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01"
UUID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
FIXED_TRACE = "0123456789abcdef0123456789abcdef"
VOLATILE = {"evaluation_context_ref", "trace_id", "request_digest", "output_digest", "input_commitment", "key_digest", "request_hash", "core_run_id", "proposal_id", "candidate_hash", "eval_run_ref", "report_digest", "jws", "exp", "deadline",
            "observed_at", "idempotency_key_digest"}


class Norm:
    def __init__(self, fixed: dict[str, str]) -> None:
        self.fixed = dict(fixed)
        self.seen: dict[tuple[str, Any], str] = {}
        self.counts: dict[str, int] = {}
        self.subst: list[tuple[str, str]] = []

    def _placeholder(self, key: str, value: Any) -> str:
        k = (key, value)
        if k not in self.seen:
            self.counts[key] = self.counts.get(key, 0) + 1
            ph = f"<{key}#{self.counts[key]}>" if key not in ("jws", "exp", "deadline", "observed_at") else f"<{key}>"
            self.seen[k] = ph
            if isinstance(value, str) and len(value) >= 8:
                self.subst.append((value, ph))
                self.subst.sort(key=lambda t: -len(t[0]))
        return self.seen[k]

    def _str(self, s: str) -> str:
        for real, ph in [*self.fixed.items(), *self.subst]:
            if real and real in s:
                s = s.replace(real, ph)
        s = UUID_RE.sub(lambda m: self._placeholder("uuid", m.group(0)), s)
        if len(s) > 200:
            return f"<string len={len(s)} sha256={hashlib.sha256(s.encode()).hexdigest()[:8]}>"
        return s

    def __call__(self, node: Any, key: str | None = None) -> Any:
        if isinstance(node, dict):
            fact = "source_kind" in node and "digest" in node  # a projected fact envelope: digest covers run ids
            return {k: self(v, "output_digest" if fact and k == "digest" else k) for k, v in node.items()}
        if isinstance(node, list):
            return [self(v, key) for v in node]
        if key == "trace_id" and node == FIXED_TRACE:
            return node
        if key in VOLATILE and isinstance(node, str | int) and not isinstance(node, bool) and node is not None:
            return self._placeholder(key, node)
        if isinstance(node, str):
            return self._str(node)
        return node


@dataclass
class Exchange:
    world: World
    flow: str
    record: bool
    norm: Norm
    n: int = 0
    out: list[dict[str, Any]] = field(default_factory=list)
    problems: list[str] = field(default_factory=list)

    def call(self, case: str, method: str, path: str, *, purpose: str, body: Any = None, tenant: Any = ...,
             job_id: str | None = "job-golden", headers: dict[str, str] | None = None, status: int,
             schema: str | None = None, raw: bytes | None = None, key: str | None = None,
             claims: dict[str, Any] | None = None, token: str | None = None, doc: str = "",
             anon: bool = False) -> Any:
        h = {"traceparent": TRACEPARENT, **(headers or {})}
        if key is not None:
            h["Idempotency-Key"] = key
        resp = self.world.api.call(method, path, purpose=None if (token or anon) else purpose, body=body, tenant=tenant,
                                   job_id=job_id, headers=h, raw=raw, claims=claims, token=token)
        who = self.world.tenant if tenant is ... else tenant
        assert resp.status_code == status, f"[{self.flow}/{case}] {resp.status_code} != {status}: {resp.text[:300]}"
        data = resp.json() if resp.content else None
        if schema:
            assert_valid(schema, data)
        self.n += 1
        step = {
            "case": case, "description": doc,
            "request": {"method": method, "path": self.norm(path),
                        "auth": {"typ": "JWT", "alg": "EdDSA", "iss": "control-api", "aud": "core-bridge",
                                 "purpose": purpose, "tenant_id": who, "job_id": job_id,
                                 "note": "token omitted: mint a fresh one per attempt (fresh jti)"}
                        if not (token or anon) else {"note": "no token" if anon else "intentionally malformed or "
                                                                                       "foreign token (not stored)"},
                        "headers": self.norm({k: v for k, v in h.items() if k.lower() != "authorization"}),
                        "body": self.norm(body) if body is not None else (
                            {"$raw": f"<{len(raw)} bytes>"} if raw is not None else None)},
            "response": {"status": resp.status_code, "schema": schema or "ErrorEnvelope",
                         "content_type": resp.headers.get("content-type", "").split(";")[0],
                         "body": self.norm(data)}}
        self.out.append(step)
        return data

    def finish(self, meta: dict[str, Any]) -> None:
        doc = {"flow": self.flow, **meta, "contract_revision": "pulso-two-teams-1", "steps": self.out}
        target = EXAMPLES / "flows" / f"{self.flow}.json"
        text = json.dumps(doc, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
        if self.record:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text, encoding="utf-8", newline="\n")
            return
        assert target.is_file(), f"golden file missing: {target} (record with CONTRACT_RECORD=1)"
        golden = json.loads(target.read_text(encoding="utf-8"))
        live = json.loads(text)
        for g, lv in zip(golden["steps"], live["steps"], strict=False):
            assert g == lv, f"[{self.flow}/{g['case']}] golden mismatch at {_first_diff(g, lv)}"
        assert len(golden["steps"]) == len(live["steps"]), "flow length differs from the golden file"


def _first_diff(a: Any, b: Any, path: str = "") -> str:
    if isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                return f"{path}/{k} (missing on one side)"
            if a[k] != b[k]:
                return _first_diff(a[k], b[k], f"{path}/{k}")
    if isinstance(a, list) and isinstance(b, list) and len(a) == len(b):
        for i, (x, y) in enumerate(zip(a, b, strict=True)):
            if x != y:
                return _first_diff(x, y, f"{path}[{i}]")
    return f"{path}: golden={json.dumps(a)[:200]} live={json.dumps(b)[:200]}"


# ------------------------------------------------------------------------------------------------------------------
def iso(hours: float = 1) -> str:
    return (datetime.now(UTC) + timedelta(hours=hours)).isoformat().replace("+00:00", "Z")


def flow_auth_and_envelope(ex: Exchange) -> dict[str, Any]:
    w = ex.world
    ex.call("version_ok", "GET", "/version", purpose="version_probe", tenant=None, job_id=None, status=200,
            schema="CoreVersion", doc="Authenticated version probe; no tenant claim is required on this route.")
    ex.call("missing_token", "GET", "/version", purpose="version_probe", anon=True, status=401,
            doc="No Authorization header: 401 pulso:auth_invalid reason=missing_token.")
    ex.call("malformed_token", "GET", "/core-tasks/run-1", purpose="core_task_read", token="x.y.z", status=401,
            doc="A syntactically wrong token is refused with a closed reason.")
    bad_sig = w.signer.sign(w.signer.claims("version_probe", w.tenant),
                            key=__import__("cryptography.hazmat.primitives.asymmetric.ed25519", fromlist=["x"])
                            .Ed25519PrivateKey.from_private_bytes(b"\x01" * 32))
    ex.call("bad_signature", "GET", "/version", purpose="version_probe", token=bad_sig, status=401,
            doc="Signature by another key.")
    ex.call("expired_token", "GET", "/version", purpose="version_probe", claims={"ttl": -5}, status=401,
            doc="exp in the past.")
    ex.call("wrong_purpose", "GET", "/core-tasks/run-1", purpose="version_probe", status=403,
            doc="Valid token, purpose not allowed on this route.")
    ex.call("tenant_not_deployed", "GET", "/core-tasks/run-1", purpose="core_task_read", tenant=w.unknown_tenant,
            status=403, doc="Tenant claim outside the deployment tenant set.")
    token = w.signer.token("version_probe", w.tenant)
    ex.call("jti_first_use", "GET", "/version", purpose="version_probe", token=token, status=200, schema="CoreVersion")
    ex.call("jti_replay", "GET", "/version", purpose="version_probe", token=token, status=401,
            doc="Second use of the same (iss, jti): 401 reason=jti_replayed.")
    ex.call("unknown_path", "GET", "/no-such-route", purpose="version_probe", status=404,
            doc="Core problem+json never appears under /internal/v1.")
    return {"description": "Auth failures, replay and the error envelope on every denial."}


def flow_invoke_scout(ex: Exchange) -> dict[str, Any]:
    w = ex.world
    w.before_scout()
    job, logical = "job-golden-scout", "golden-scout-1"
    body = w.invocation("scout", job, logical, extract_manifest_ref="ex-1")
    key = idempotency_key(w.tenant, job, "scout", 1, logical)
    first = ex.call("invoke_scout_ok", "POST", "/core-tasks/invoke", purpose="core_task_invoke", body=body, key=key,
                    job_id=job, status=200, schema="CoreTaskReceipt",
                    doc="Scout stage, terminal_ok, one whitelisted fact (pulso_hypotheses).")
    ex.call("invoke_scout_replay", "POST", "/core-tasks/invoke", purpose="core_task_invoke", body=body, key=key,
            job_id=job, status=200, schema="CoreTaskReceipt", doc="Same key + body: the stored receipt, no new run.")
    ex.call("invoke_digest_conflict", "POST", "/core-tasks/invoke", purpose="core_task_invoke",
            body={**body, "input": {"briefing_ref": "wiki/other.md"}}, key=key, job_id=job, status=409,
            doc="Same Idempotency-Key, different body.")
    ex.call("read_task_ok", "GET", f"/core-tasks/{first['core_run_id']}", purpose="core_task_read", job_id=job,
            status=200, schema="CoreTaskReceipt")
    ex.call("read_task_foreign_tenant", "GET", f"/core-tasks/{first['core_run_id']}", purpose="core_task_read",
            tenant=w.other_tenant, job_id=job, status=404, doc="A foreign tenant's run is indistinguishable from missing.")
    ex.call("read_task_unknown", "GET", "/core-tasks/run-does-not-exist", purpose="core_task_read", job_id=job, status=404)
    bad = {**body, "logical_key": "golden-scout-2", "release_id": "rel-does-not-exist"}
    ex.call("invoke_release_unavailable", "POST", "/core-tasks/invoke", purpose="core_task_invoke", body=bad,
            key=idempotency_key(w.tenant, job, "scout", 1, "golden-scout-2"), job_id=job, status=409,
            doc="Release pin cannot be proven: 409 pulso:release_pin_unavailable, receipt terminal_failed.")
    for case, over, code_status in (("invoke_unknown_stage", {"stage": "nope"}, 400),
                                    ("invoke_stage_agent_mismatch", {"agent_id": "pulso-writer"}, 422),
                                    ("invoke_unknown_input_slot", {"input": {"briefing_ref": "a", "zzz": 1}}, 400),
                                    ("invoke_unknown_field", {"surprise": 1}, 422)):
        b = {**body, "logical_key": f"golden-{case}", **over}
        ex.call(case, "POST", "/core-tasks/invoke", purpose="core_task_invoke", body=b,
                key=idempotency_key(w.tenant, job, b["stage"], 1, b["logical_key"]), job_id=job, status=code_status)
    ex.call("invoke_wrong_idempotency_key", "POST", "/core-tasks/invoke", purpose="core_task_invoke", body=body,
            key="not-the-formula", job_id=job, status=422, doc="Idempotency-Key must equal the published formula.")
    big = {**body, "logical_key": "golden-big", "input": {"briefing_ref": "x" * 262_200}}
    ex.call("invoke_input_too_large", "POST", "/core-tasks/invoke", purpose="core_task_invoke", body=big,
            key=idempotency_key(w.tenant, job, "scout", 1, "golden-big"), job_id=job, status=413)
    return {"description": "Scout invocation: receipt state machine, replay, conflict, read, refusals."}


def flow_credentials(ex: Exchange) -> dict[str, Any]:
    w = ex.world
    for role, purpose in (("constructor", "registry_write"), ("constructor", "core_task")):
        ex.call(f"issue_{purpose}", "POST", "/core-credentials/issue", purpose="credential_issue",
                body={"tenant_id": w.tenant, "role": role, "purpose": purpose}, status=200,
                schema="CoreCredentialIssue", doc="Principal JWS typ=principal+jws; value redacted in the example.")
    ex.call("issue_not_issuable", "POST", "/core-credentials/issue", purpose="credential_issue",
            body={"tenant_id": w.tenant, "role": "approver", "purpose": "registry_write"}, status=403)
    ex.call("issue_tenant_mismatch", "POST", "/core-credentials/issue", purpose="credential_issue",
            body={"tenant_id": w.other_tenant, "role": "constructor", "purpose": "registry_write"}, status=403)
    ex.call("issue_invalid_request", "POST", "/core-credentials/issue", purpose="credential_issue",
            body={"tenant_id": w.tenant, "role": "constructor"}, status=422)
    return {"description": "Bot credential issuance (policy table) and its refusals."}


def flow_writer_evaluation(ex: Exchange) -> dict[str, Any]:
    w = ex.world
    plan = w.writer_plan("golden")
    job, logical = "job-golden-writer", "golden-writer-1"
    body = w.invocation("writer", job, logical, input={
        "draft_plan_ref": plan["plan_ref"], "proposal_id": None, "base_release_id": plan["base_release_id"],
        "evaluate_enabled": False}, registry_mutation_commitment=plan["commitment"])
    wr = ex.call("writer_create_put_freeze", "POST", "/core-tasks/invoke", purpose="core_task_invoke", body=body,
                 key=idempotency_key(w.tenant, job, "writer", 1, logical), job_id=job, status=200,
                 schema="CoreTaskReceipt", doc="Writer with a sealed commitment: create_proposal, put_draft, freeze.")
    facts = wr["result"]["facts"]["pulso_writer_receipts"]["value"]
    assert_valid("Fact_pulso_writer_receipts", facts)
    cand = {"proposal_id": facts["proposal_id"], "candidate_hash": facts["candidate_hash"]}
    ejob, elogical = "job-golden-eval", "golden-eval-1"
    ekey = idempotency_key(w.tenant, ejob, "writer", 1, elogical)
    bref = hashlib.sha256(f"{w.tenant}|{ekey}".encode()).hexdigest()
    adm = {"schema_version": "1", "binding_ref": bref, **cand,
           "suite_id": plan["suite_id"], "suite_version": plan["suite_version"], "suite_digest": plan["suite_digest"],
           "evaluation_attempt": 1, "budget_ref": w.budget_ref, "deadline": iso(), "request_digest": "d" * 64}
    ref = evaluation_context_ref(w.tenant, ejob, bref, cand["proposal_id"], cand["candidate_hash"], 1)
    ex.call("admission_created", "POST", "/evaluation/admissions", purpose="evaluation_admit", body=adm, job_id=ejob,
            status=201, schema="EvaluationAdmission", doc="First admission: 201 state=admitted.")
    ex.call("admission_replay", "POST", "/evaluation/admissions", purpose="evaluation_admit", body=adm, job_id=ejob,
            status=200, schema="EvaluationAdmission", doc="Identical replay: 200, same body.")
    ex.call("admission_conflict", "POST", "/evaluation/admissions", purpose="evaluation_admit",
            body={**adm, "request_digest": "e" * 64}, job_id=ejob, status=409,
            doc="Same derived context ref, different request digest: 409 pulso:idempotency_conflict.")
    ex.call("admission_explicit_ref_equal", "POST", "/evaluation/admissions", purpose="evaluation_admit",
            body={**adm, "evaluation_context_ref": ref}, job_id=ejob, status=200, schema="EvaluationAdmission",
            doc="A deprecated explicit ref equal to the derived one is accepted (replay).")
    ex.call("admission_stale_candidate", "POST", "/evaluation/admissions", purpose="evaluation_admit",
            body={**adm, "candidate_hash": "0" * 64}, job_id=ejob, status=409)
    ex.call("admission_bad_context_ref", "POST", "/evaluation/admissions", purpose="evaluation_admit",
            body={**adm, "evaluation_context_ref": "has space"}, job_id=ejob, status=422)
    eb = w.invocation("writer", ejob, elogical, input={
        "draft_plan_ref": plan["plan_ref"], "proposal_id": cand["proposal_id"],
        "base_release_id": plan["base_release_id"], "evaluate_enabled": True,
        "evaluation_suite_id": plan["suite_id"], "evaluation_suite_version": plan["suite_version"]},
        registry_mutation_commitment={"mode": "evaluate_only", "proposal_id": cand["proposal_id"],
                                      "base_release_id": plan["base_release_id"], "evaluate_enabled": True,
                                      "evaluation_context_ref": ref, "operations": []})
    er = ex.call("invoke_evaluate_only", "POST", "/core-tasks/invoke", purpose="core_task_invoke", body=eb, key=ekey,
                 job_id=ejob, status=200, schema="CoreTaskReceipt",
                 doc="Evaluate-only writer under the admission: native evaluation report digest in the fact.")
    ev = er["result"]["facts"]["pulso_writer_receipts"]["value"]
    assert_valid("Fact_pulso_writer_receipts", ev)
    assert ev["native_evaluation"]["verdict"] == "pass"
    return {"description": "Frozen candidate -> evaluation admission -> evaluate-only invocation (native report)."}


def flow_arms(ex: Exchange) -> dict[str, Any]:
    w = ex.world
    w.seal_manifest("art-golden")
    key = "golden-arm-1"
    body = {"idempotency_key": key, "binding_ref": "bind-arm", "campaign_ref": "camp-1", "case_ref": "case-1",
            "arm": "baseline", "repetition": 0, "seed": 7, "mode": "native", "agent_id": w.arm_agent,
            "target": {"kind": "published_release", "release_id": w.arm_release},
            "scenario_manifest_ref": "art-golden", "budget_ref": w.budget_ref, "seed_manifest_ref": None}
    rep = ex.call("arm_run_native", "POST", "/evaluation/arms/run", purpose="evaluation_arm_run", body=body, status=200,
                  schema="ArmReport", doc="Native arm on the published release; execution_id = arm-sha256(tenant|key)[:32].")
    ex.call("arm_run_replay", "POST", "/evaluation/arms/run", purpose="evaluation_arm_run", body=body, status=200,
            schema="ArmReport", doc="Same key + digest: the stored report is returned.")
    ex.call("arm_run_conflict", "POST", "/evaluation/arms/run", purpose="evaluation_arm_run", body={**body, "seed": 8},
            status=409, doc="Same key, different body.")
    ex.call("arm_read_by_id", "GET", f"/evaluation/arms/{rep['execution_id']}", purpose="evaluation_arm_read",
            status=200, schema="ArmReport")
    ex.call("arm_read_by_key", "GET", f"/evaluation/arms/by-key/{key}", purpose="evaluation_arm_read", status=200,
            schema="ArmReport")
    ex.call("arm_read_unknown", "GET", "/evaluation/arms/by-key/never-seen", purpose="evaluation_arm_read", status=404)
    ex.call("arm_read_foreign_tenant", "GET", f"/evaluation/arms/{rep['execution_id']}", purpose="evaluation_arm_read",
            tenant=w.other_tenant, status=404)
    ex.call("arm_mixed_world", "POST", "/evaluation/arms/run", purpose="evaluation_arm_run",
            body={**body, "idempotency_key": "golden-arm-mixed", "seed_manifest_ref": "seed-1"}, status=409)
    ex.call("arm_sandbox_required", "POST", "/evaluation/arms/run", purpose="evaluation_arm_run",
            body={**body, "idempotency_key": "golden-arm-nosb", "mode": "task_builder"}, status=409)
    ex.call("arm_oracle_field_rejected", "POST", "/evaluation/arms/run", purpose="evaluation_arm_run",
            body={**body, "idempotency_key": "golden-arm-gold", "oracle_gold": {"answer": 1}}, status=422,
            doc="Gold/oracle fields can never ride along: the DTO is closed.")
    ex.call("arm_budget_unknown", "POST", "/evaluation/arms/run", purpose="evaluation_arm_run",
            body={**body, "idempotency_key": "golden-arm-nobud", "budget_ref": "bud-missing-xyz"}, status=200,
            schema="ArmReport", doc="Failure inside the run is REPORTED (200, status=failed_infra), not raised.")
    return {"description": "Arm run, replay, readback by id/key, closed refusals, in-run failure."}


def flow_authoring(ex: Exchange) -> dict[str, Any]:
    w = ex.world
    ex.call("alias_read", "GET", f"/core-state/aliases/{w.arm_agent}/prod", purpose="alias_read", status=200,
            schema="AliasState")
    ex.call("alias_unknown_agent", "GET", "/core-state/aliases/no-such-agent/prod", purpose="alias_read", status=404)
    ex.call("alias_bad_name", "GET", f"/core-state/aliases/{w.arm_agent}/canary", purpose="alias_read", status=422)
    base = {"schema_version": "1", "tenant_id": w.tenant, "agent_id": w.arm_agent, "base_release_id": None,
            "changes": []}
    ex.call("dry_run_valid", "POST", "/core-authoring/dry-run", purpose="authoring_dry_run",
            body={**base, "base_release_id": w.arm_release}, status=200, schema="CoreAuthoringDryRun",
            doc="The base release alone is a valid candidate: valid=true, candidate_hash set, no proposal created.")
    ex.call("dry_run_base_release_unknown", "POST", "/core-authoring/dry-run", purpose="authoring_dry_run",
            body={**base, "base_release_id": "rel-nope"}, status=404)
    ex.call("dry_run_violations", "POST", "/core-authoring/dry-run", purpose="authoring_dry_run",
            body={**base, "changes": [{"kind": "no-such-kind", "content": {}, "docs": {}}]}, status=200,
            schema="CoreAuthoringDryRun", doc="HTTP 200 with violations is NOT success: candidate_hash is null.")
    ex.call("dry_run_tenant_mismatch", "POST", "/core-authoring/dry-run", purpose="authoring_dry_run",
            body={**base, "tenant_id": w.other_tenant}, status=403)
    ex.call("dry_run_unknown_field", "POST", "/core-authoring/dry-run", purpose="authoring_dry_run",
            body={**base, "surprise": 1}, status=422)
    return {"description": "Alias read (CAP-08) and authoring dry-run (CAP-16 L2): 200/404/422 alias cases, a valid "
                           "dry-run (base release alone, candidate_hash set), violations and request errors."}


FLOWS: dict[str, tuple[Callable[[Exchange], dict[str, Any]], set[str]]] = {
    "auth_and_envelope": (flow_auth_and_envelope, set()),
    "invoke_scout": (flow_invoke_scout, {"invoke"}),
    "credentials": (flow_credentials, {"credentials"}),
    "writer_evaluation": (flow_writer_evaluation, {"writer", "evaluation"}),
    "arms": (flow_arms, {"evaluation", "arms"}),
    "authoring": (flow_authoring, {"authoring"}),
}


def run_flow(world: World, name: str, record: bool) -> None:
    fn, needs = FLOWS[name]
    missing = needs - world.caps
    if missing:
        pytest.skip(f"target '{world.target}' lacks capabilities {sorted(missing)}")
    fixed = {world.scout_release: "<release:scout>", world.writer_release: "<release:writer>"}
    ex = Exchange(world, name, record, Norm(fixed))
    meta = fn(ex)
    ex.finish(meta)
