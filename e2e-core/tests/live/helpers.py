"""The happy-path pipeline, executed ONCE per session against the real_local stack (scripted model)."""

from __future__ import annotations

import hashlib
import time
from types import SimpleNamespace
from typing import Any

import yaml

from codex_standin.dto import admission, digest_json, idempotency_key
from codex_standin.engine import (
    ASSETS,
    DESIGN,
    RESEARCH,
    TENANT,
    VERIFY,
    Engine,
    candidate_changes,
    change_spec_output,
    hypotheses_output,
    put_draft_digest,
    suite_digest,
    verification_output,
)

OPS = ["create_proposal", "put_draft", "freeze"]


def tag() -> str:
    return str(time.time_ns())[-12:]


def drafts_digest(changes: list[dict[str, Any]]) -> str:
    from agent_core.registry.models import EntityDraft
    drafts = [EntityDraft.model_validate(c) for c in changes]
    ordered = sorted(drafts, key=lambda d: (d.kind, str(d.content.get("id", ""))))
    return digest_json([d.model_dump(mode="json") for d in ordered])


def writer_commitment(base_rel: str, title: str, changes: list[dict[str, Any]]) -> dict[str, Any]:
    return {"mode": "write", "base_release_id": base_rel, "create_agent_id": "pulso-scout",
            "create_origin": "builder_chat", "create_title": title,
            "put_draft_digest": put_draft_digest(None, None, changes), "operations": OPS}


def smoke_scenarios() -> list[dict[str, Any]]:
    suite = yaml.safe_load((ASSETS / "worlds/pulso-evolution/eval_suites/pulso-smoke@1.0.0.yaml").read_text("utf-8"))
    return suite["scenarios"]  # type: ignore[no-any-return]


def arm_body(key: str, binding_ref: str, manifest: str, mode: str, target: dict[str, Any],
             **over: Any) -> dict[str, Any]:
    return {"idempotency_key": key, "binding_ref": binding_ref, "campaign_ref": "camp-e2e", "case_ref": "case-e2e",
            "arm": "baseline", "repetition": 0, "seed": 7, "mode": mode, "agent_id": "pulso-scout", "target": target,
            "scenario_manifest_ref": manifest, "budget_ref": "bud-e2e",
            "seed_manifest_ref": None if mode == "native" else "seed-e2e", **over}


def run_arm(e: Engine, body: dict[str, Any], tenant: str = TENANT) -> SimpleNamespace:
    r = e.bridge.arm_run(tenant, body)
    return SimpleNamespace(body=body, response=r, report=r.json() if r.status_code == 200 else None)


def run_pipeline(e: Engine, db: Any) -> SimpleNamespace:
    n = tag()
    p = SimpleNamespace(n=n, tenant=TENANT)
    rel = e.releases
    e.script_stage_model(f"scout-{n}", RESEARCH, hypotheses_output(TENANT))
    e.script_stage_model(f"verifier-{n}", VERIFY, verification_output(TENANT))
    e.script_stage_model(f"design-{n}", DESIGN, change_spec_output(TENANT))
    # -- scout
    p.scout = e.stage("scout", f"job-scout-{n}", "scout", "pulso-scout", {"briefing_ref": f"wiki/briefing-{n}.md"},
                      memory_snapshot_ref=f"mem-{n}", extract_manifest_ref=f"ex-{n}")
    p.scout_facts = e.facts(p.scout.out["core_run_id"]) if p.scout.out.get("core_run_id") else {}
    # -- verifier: a DIFFERENT agent/release/job; its input is the sealed hypotheses of the scout
    hyp = p.scout_facts.get("pulso_hypotheses", {}).get("value", {})
    e.seal(f"hyp-{n}", hyp)
    p.verifier = e.stage("verifier", f"job-verifier-{n}", "verifier", "pulso-verifier",
                         {"hypotheses_ref": f"hyp-{n}"}, memory_snapshot_ref=f"mem-{n}",
                         extract_manifest_ref=f"ex-{n}")
    p.verifier_facts = e.facts(p.verifier.out["core_run_id"]) if p.verifier.out.get("core_run_id") else {}
    # -- builder_design
    ver = p.verifier_facts.get("pulso_verification", {}).get("value", {})
    e.seal(f"design-in-{n}", {"hypotheses": hyp.get("hypotheses"), "assessments": ver.get("assessments")})
    p.design = e.stage("builder_design", f"job-design-{n}", "design", "pulso-builder-design",
                       {"design_input_ref": f"design-in-{n}"}, memory_snapshot_ref=f"mem-{n}",
                       extract_manifest_ref=f"ex-{n}")
    p.design_facts = e.facts(p.design.out["core_run_id"]) if p.design.out.get("core_run_id") else {}
    # -- writer (create_proposal, put_draft, validate, freeze; evaluate_enabled=false)
    p.changes = candidate_changes()
    p.title = f"pulso-key:e2e-{n}"
    e.seal(f"plan-{n}", {"agent_id": "pulso-scout", "title": p.title, "changes": p.changes})
    p.base_rel = rel["pulso-scout"]
    p.writer = e.stage("writer", f"job-writer-{n}", "writer", "pulso-writer", {
        "draft_plan_ref": f"plan-{n}", "proposal_id": None, "base_release_id": p.base_rel, "evaluate_enabled": False},
        registry_mutation_commitment=writer_commitment(p.base_rel, p.title, p.changes))
    p.writer_facts = e.facts(p.writer.out["core_run_id"]) if p.writer.out.get("core_run_id") else {}
    wr = p.writer_facts.get("pulso_writer_receipts", {}).get("value", {})
    p.proposal_id, p.candidate_hash = wr.get("proposal_id"), wr.get("candidate_hash")
    p.suite_digest = suite_digest(p.changes)
    # -- evaluation admission, then the evaluate-only invocation
    p.ctx_ref = f"ctx-{n}"
    # The admission is bound to the evaluate-only invocation's OWN (job_id, binding_ref): the runtime derives that
    # binding_ref as sha256_text("tenant|Idempotency-Key") before the invocation runs, so the stand-in computes it.
    p.eval_job = f"job-evalonly-{n}"
    p.eval_key = idempotency_key(TENANT, p.eval_job, "writer", 1, "evalonly")
    p.eval_binding_ref = hashlib.sha256(f"{TENANT}|{p.eval_key}".encode()).hexdigest()
    p.admission_body = admission(ref=p.ctx_ref, binding_ref=p.eval_binding_ref,
                                 proposal_id=p.proposal_id, candidate_hash=p.candidate_hash, suite_id="pulso-smoke",
                                 suite_version="1.1.0", suite_digest=p.suite_digest, budget_ref="bud-e2e")
    # control-api double: the platform issued the evaluate-only task's binding_ref (derivable from tenant|key) before the
    # evaluation admission is requested; the runtime's own bind callback later confirms the same ref.
    e.configure(preauthorized_bindings=[{"tenant": TENANT, "binding_ref": p.eval_binding_ref}])
    p.admit = e.bridge.admit(TENANT, p.eval_job, p.admission_body)
    p.admit_replay = e.bridge.admit(TENANT, p.eval_job, p.admission_body)  # before the evaluate-only run consumes it
    p.eval_only = e.stage("writer", p.eval_job, "evalonly", "pulso-writer", {
        "draft_plan_ref": f"plan-{n}", "proposal_id": p.proposal_id, "base_release_id": p.base_rel,
        "evaluate_enabled": True, "evaluation_suite_id": "pulso-smoke", "evaluation_suite_version": "1.1.0"},
        registry_mutation_commitment={"mode": "evaluate_only", "proposal_id": p.proposal_id,
                                      "base_release_id": p.base_rel, "evaluate_enabled": True,
                                      "evaluation_context_ref": p.ctx_ref, "operations": []})
    assert p.eval_only.key == p.eval_key
    p.eval_facts = e.facts(p.eval_only.out["core_run_id"]) if p.eval_only.out.get("core_run_id") else {}
    # -- arms with the fixture bank: published base (native + task_builder) and the frozen candidate
    scen = smoke_scenarios()
    e.seal(f"manifest-{n}", {"scenarios": scen, "entries": {s["id"]: {} for s in scen}})
    p.manifest = f"manifest-{n}"
    bref = p.writer.out["task_binding_ref"]
    p.arm_native = run_arm(e, arm_body(f"arm-native-{n}", bref, p.manifest, "native",
                                       {"kind": "published_release", "release_id": p.base_rel}))
    p.arm_bank = run_arm(e, arm_body(f"arm-bank-{n}", bref, p.manifest, "task_builder",
                                     {"kind": "published_release", "release_id": p.base_rel}))
    rev = int(db.one("select (proposal_json::json->>'rev')::int from reg_proposals where proposal_id=%s",
                     p.proposal_id))
    p.cand_target = {"kind": "frozen_candidate", "proposal_id": p.proposal_id, "expected_rev": rev,
                     "base_release_id": p.base_rel, "candidate_hash": p.candidate_hash,
                     "draft_plan_digest": drafts_digest(p.changes)}
    p.arm_cand = run_arm(e, arm_body(f"arm-cand-{n}", bref, p.manifest, "task_builder", p.cand_target,
                                     arm="candidate"))
    return p
