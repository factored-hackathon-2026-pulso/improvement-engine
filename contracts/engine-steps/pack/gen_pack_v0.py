"""One-shot generator of the FRZ0 pack parts (run once; the files are the contract).

python gen_pack_v0.py && python verify_pack.py --write
Goldens of parts 6 and 7 are byte copies of bridge-contract/examples/flows/*.
"""
import json
import shutil
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import gen_draft_v0 as g  # noqa: E402  (regenerates draft v0 files as a side effect)

REPO = HERE.parents[2]
P = HERE / "parts"


def dump(rel, obj):
    p = P / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(obj, indent=2) + "\n", encoding="utf-8", newline="\n")


REF, ID, DG = g.REF, g.ID, g.DG
SD = g.SD
CV = {"const": "engine-steps-pack/0"}

# ---- part 1: C-7 v1.1 claim-next
dump("c7_claim_next/claim_next.v1_1.json", {
    "contract_version": "engine-steps-pack/0", "contract": "C-7", "version": "v1.1",
    "trait": "DurableJobRepository", "base_version": "v1 (existing trait, frozen at CF0)",
    "rust_signature": (
        "fn claim_next_job(&mut self, tenant_id: &str, worker_id: &str, "
        "now_unix_seconds: u64, lease_seconds: u64) -> Result<Option<ClaimedJob>, DurableJobError>;"),
    "returns": {"ClaimedJob": {"job_id": "String", "lease": "JobLease{worker_id,fence_token,expires_at_unix_seconds}"}},
    "semantics": [
        "tenant-scoped: never claims or reveals another tenant's job",
        "candidate = oldest admitted job (admission order) in status Queued, or Leased with an expired lease",
        "skips jobs with effect_state != NoEffect (UnknownPendingReconciliation needs reconciliation)",
        "skips Paused, CancelledBeforeEffect, CompletedNoEffect and AppliedAcknowledged jobs",
        "selection and lease are one conditional update (no read-then-write); two workers never claim the same job",
        "a claim increments fence_token and attempt_count by one and bumps control_version once",
        "no candidate: Ok(None), no state change",
        "lease_seconds == 0 is InvalidLeaseDuration; invalid worker_id or tenant_id is rejected before any read",
    ],
    "implemented_by": "DJC (Codex, X-CONF); this file only publishes the signature",
})
STEP = lambda op, **k: {"op": op, **k}  # noqa: E731
traces = {
    "claim_oldest_first": ("equivalent_triggers_admit_one_job_and_reserve_quota_once", [
        STEP("admit", trigger="t-1", expect={"admitted": True, "job": "job-a"}),
        STEP("admit", trigger="t-2", expect={"admitted": True, "job": "job-b"}),
        STEP("claim_next_job", tenant="tenant-a", worker="w1", now=100, lease=30,
             expect={"job": "job-a", "fence_token": 1, "attempt": 1, "expires_at": 130}),
        STEP("claim_next_job", tenant="tenant-a", worker="w2", now=101, lease=30,
             expect={"job": "job-b", "fence_token": 1, "attempt": 1, "expires_at": 131}),
        STEP("claim_next_job", tenant="tenant-a", worker="w3", now=102, lease=30, expect=None)]),
    "expired_lease_reclaimed_with_higher_fence": (
        "stale_worker_cannot_dispatch_or_acknowledge_after_its_lease_is_replaced", [
            STEP("admit", trigger="t-1", expect={"admitted": True, "job": "job-a"}),
            STEP("claim_next_job", tenant="tenant-a", worker="w1", now=100, lease=30,
                 expect={"job": "job-a", "fence_token": 1}),
            STEP("claim_next_job", tenant="tenant-a", worker="w2", now=110, lease=30, expect=None),
            STEP("claim_next_job", tenant="tenant-a", worker="w2", now=130, lease=30,
                 expect={"job": "job-a", "fence_token": 2, "attempt": 2}),
            STEP("begin_job_effect_dispatch", worker="w1", fence_token=1, now=131,
                 expect={"error": "stale_fence"})]),
    "tenant_isolation": ("tenant_cannot_read_or_transition_another_tenants_job", [
        STEP("admit", tenant="tenant-a", trigger="t-1", expect={"admitted": True, "job": "job-a"}),
        STEP("claim_next_job", tenant="tenant-b", worker="w1", now=100, lease=30, expect=None),
        STEP("claim_next_job", tenant="tenant-a", worker="w1", now=100, lease=30,
             expect={"job": "job-a", "fence_token": 1})]),
    "unknown_effect_not_claimable": (
        "crash_after_dispatch_is_unknown_and_recovery_never_releases_it_for_retry", [
            STEP("admit", trigger="t-1", expect={"admitted": True, "job": "job-a"}),
            STEP("claim_next_job", tenant="tenant-a", worker="w1", now=100, lease=30,
                 expect={"job": "job-a", "fence_token": 1}),
            STEP("begin_job_effect_dispatch", worker="w1", fence_token=1, now=101, expect="ok"),
            STEP("recover_job_after_restart", now=200, expect={"status": "UnknownPendingReconciliation"}),
            STEP("claim_next_job", tenant="tenant-a", worker="w2", now=201, lease=30, expect=None)]),
    "paused_and_deferred_not_claimable": (
        "pause_is_an_atomic_durable_transition_with_idempotent_audit_receipt", [
            STEP("admit", trigger="t-1", expect={"admitted": True, "job": "job-a"}),
            STEP("control_job_atomically", kind="pause", expect={"status": "Paused"}),
            STEP("claim_next_job", tenant="tenant-a", worker="w1", now=100, lease=30, expect=None),
            STEP("admit", trigger="t-over-quota", expect={"admitted": False, "job": None}),
            STEP("claim_next_job", tenant="tenant-a", worker="w1", now=100, lease=30, expect=None)]),
    "invalid_lease_duration": ("(error path of acquire_lease)", [
        STEP("admit", trigger="t-1", expect={"admitted": True, "job": "job-a"}),
        STEP("claim_next_job", tenant="tenant-a", worker="w1", now=100, lease=0,
             expect={"error": "InvalidLeaseDuration"})]),
}
for name, (test, steps) in traces.items():
    dump(f"c7_claim_next/traces/{name}.json", {
        "contract_version": "engine-steps-pack/0", "name": name,
        "kind": "specification_trace", "recorded": False,
        "derived_from_test": "crates/core/tests/durable_jobs.rs::" + test,
        "note": ("Derived by reading the in-memory reducer tests (cargo is not run by the pack "
                 "author); a PG-recorded variant replaces `recorded` with true when DJC runs."),
        "steps": steps})

# ---- part 2: C-8 schemas
CS = dict(g.CHANGE_SPEC)
CS.update({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "ChangeSpec",
           "title": "ChangeSpec (C-8 draft, SMAP refines)"})
dump("c8_schemas/ChangeSpec.schema.json", CS)
SC = g.obj(
    ["contract_version", "case_id", "agent_id", "route", "seed", "turns", "initial_state_digest",
     "oracle_ref", "data_class"],
    {"contract_version": CV, "case_id": ID, "agent_id": ID,
     "route": {"type": "string", "minLength": 1},
     "seed": {"type": "integer", "minimum": 0},
     "turns": g.arr(g.obj(["role", "text"], {"role": {"enum": ["customer", "agent"]},
                                              "text": {"type": "string", "minLength": 1}}), 1),
     "initial_state_digest": {"type": ["string", "null"], "pattern": g.SHA},
     "oracle_ref": {"type": ["string", "null"]},
     "data_class": {"const": "synthetic"}})
SC.update({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "ScenarioCase",
           "title": "ScenarioCase (C-8 draft)"})
dump("c8_schemas/ScenarioCase.schema.json", SC)
valid_cs = json.loads((HERE.parent / "samples/valid-compile.in.json").read_text("utf-8"))["change_spec"]
dump("c8_schemas/valid-ChangeSpec.json", valid_cs)
bad_cs = json.loads(json.dumps(valid_cs))
bad_cs["operations"][0]["precondition_digest"] = "latest"
dump("c8_schemas/invalid-ChangeSpec.json", bad_cs)
valid_sc = {"contract_version": "engine-steps-pack/0", "case_id": "case-0001", "agent_id": "atencion",
            "route": "pqr_status_lookup", "seed": 7,
            "turns": [{"role": "customer", "text": "Where is my complaint?"}],
            "initial_state_digest": SD, "oracle_ref": None, "data_class": "synthetic"}
dump("c8_schemas/valid-ScenarioCase.json", valid_sc)
bad_sc = dict(valid_sc, turns=[], seed=-1)
dump("c8_schemas/invalid-ScenarioCase.json", bad_sc)

# ---- part 3: C-9 transcript seed
steps10 = [
    (1, "data_wakes_engine", "manual_command", "e0"),
    (2, "signals_and_discards", "real-narrow", "e0"),
    (3, "scout_and_verifier", "agent_roleplay", "treated"),
    (4, "opportunity_and_alternatives", "agent_roleplay", "treated"),
    (5, "concrete_change", "claude-standin", "synthetic"),
    (6, "base_vs_candidate_gates", "real-narrow", "synthetic"),
    (7, "bounded_revision", "not_exercised", "synthetic"),
    (8, "human_authority", "simulated", "synthetic"),
    (9, "staging_alias_read", "real-narrow", "synthetic"),
    (10, "observations_and_memory", "not_exercised", "synthetic")]
dump("c9_transcript/transcript.seed.json", {
    "contract_version": "engine-steps-pack/0", "contract": "C-9", "revision": "seed-1",
    "note": "Seed only: step order, status vocabulary and data class per step; X-FACADE and L-GOV extend it.",
    "status_vocabulary": ["manual_command", "real-narrow", "agent_roleplay", "simulated",
                          "claude-standin", "not_exercised"],
    "events": [{"step": n, "name": nm, "status": st, "data_class": dc} for n, nm, st, dc in steps10]})

# ---- part 4: C-10 gate result
gate_branch = lambda v, st: {"properties": {  # noqa: E731
    "verdict": {"const": v},
    "gates": {"items": {"properties": {"status": {"const": st}}}}}}
GR = g.obj(
    ["contract_version", "run_id", "base_ref", "candidate_ref", "suite_digest", "verdict",
     "gates", "judge_actor", "quality_claims"],
    {"contract_version": CV, "run_id": ID, "base_ref": REF, "candidate_ref": REF,
     "suite_digest": DG, "verdict": {"enum": ["pass", "fail", "not_evaluable"]},
     "gates": g.arr(g.obj(["gate", "status"], {
         "gate": {"enum": ["safety", "improvement"]},
         "status": {"enum": ["pass", "fail", "not_evaluable"]},
         "reason": {"type": "string"}}), 2),
     "judge_actor": ID, "quality_claims": {"const": "forbidden"}})
GR.update({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "GateResult",
           "title": "Gate result shape (C-10); semantics are filled by Codex",
           "oneOf": [gate_branch("pass", "pass"), {"properties": {"verdict": {"const": "fail"}}},
                     {"properties": {"verdict": {"const": "not_evaluable"}}}]})
dump("c10_gate_result/gate_result.schema.json", GR)
vp = {"contract_version": "engine-steps-pack/0", "run_id": "run-demo-0001",
      "base_ref": "bundle:sample-1@1", "candidate_ref": "bundle:sample-2@1", "suite_digest": SD,
      "verdict": "pass", "gates": [{"gate": "safety", "status": "pass"},
                                   {"gate": "improvement", "status": "pass"}],
      "judge_actor": "actor-judge", "quality_claims": "forbidden"}
dump("c10_gate_result/valid-pass.json", vp)
ip = json.loads(json.dumps(vp))
ip["gates"][1]["status"] = "fail"
dump("c10_gate_result/invalid-pass-with-failed-gate.json", ip)

# ---- part 5: builder corpus
BO = g.obj(
    ["contract_version", "output_id", "data_class", "finding_ref", "evidence_refs", "design_intent"],
    {"contract_version": CV, "output_id": ID, "data_class": {"const": "synthetic"},
     "finding_ref": ID, "evidence_refs": g.arr(ID),
     "design_intent": g.obj(["verdict", "mechanism"], {
         "verdict": {"enum": ["linked", "unlinked", "not_evaluable", "do_nothing"]},
         "target_ref": {"type": ["string", "null"]},
         "mechanism": {"type": "string", "minLength": 1},
         "affected_routes": g.arr({"type": "string", "minLength": 1})})})
BO.update({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "BuilderOutput",
           "title": "Builder output (synthetic corpus v0)"})
dump("builder_corpus/builder_output.schema.json", BO)
dump("builder_corpus/catalogue.json", {
    "contract_version": "engine-steps-pack/0", "data_class": "synthetic",
    "evidence_refs": ["ev-0001", "ev-0002", "ev-0003", "ev-0004"],
    "target_refs": ["prompt:pqr-followup@1", "prompt:status-lookup@1", "eval_suite:pqr-core@1"]})
corp = [
    ("bo-01-linked-prompt", "valid", False, ["ev-0001"], "linked", "prompt:pqr-followup@1", ["pqr_status_lookup"]),
    ("bo-02-linked-suite", "valid", False, ["ev-0002", "ev-0003"], "linked", "eval_suite:pqr-core@1", ["pqr_status_lookup"]),
    ("bo-03-do-nothing", "valid", False, ["ev-0004"], "do_nothing", None, []),
    ("bo-04-linked-second-prompt", "valid", True, ["ev-0001"], "linked", "prompt:status-lookup@1", ["transaction_status_lookup"]),
    ("bo-05-unlinked", "unlinked", False, ["ev-0002"], "unlinked", None, []),
    ("bo-06-unlinked-no-route", "unlinked", False, ["ev-0003"], "unlinked", None, []),
    ("bo-07-not-evaluable", "not_evaluable", False, ["ev-0001"], "not_evaluable", None, ["pqr_status_lookup"]),
    ("bo-08-not-evaluable-empty-evidence", "not_evaluable", False, [], "not_evaluable", None, []),
    ("bo-09-invented-evidence", "invalid", True, ["ev-9999"], "linked", "prompt:pqr-followup@1", ["pqr_status_lookup"]),
    ("bo-10-invented-target", "invalid", True, ["ev-0001"], "linked", "prompt:does-not-exist@1", ["pqr_status_lookup"]),
]
verd = {}
for oid, v, needs, ev, intent, tgt, routes in corp:
    verd[oid] = v
    dump(f"builder_corpus/outputs/{oid}.json", {
        "contract_version": "engine-steps-pack/0", "output_id": oid, "data_class": "synthetic",
        "finding_ref": "sig-0001", "evidence_refs": ev,
        "design_intent": {"verdict": intent, "target_ref": tgt,
                          "mechanism": "synthetic mechanism text for " + oid,
                          "affected_routes": routes}})
dump("builder_corpus/verdicts.json", {
    "contract_version": "engine-steps-pack/0",
    "rule": ("valid = refs resolve in catalogue.json; unlinked and not_evaluable are decided "
             "without the catalogue; invalid = any evidence or target ref absent from the catalogue"),
    "verdicts": verd,
    "needs_catalogue": {o: n for o, _, n, *_ in corp}})

# ---- parts 6 and 7: byte copies of bridge-contract goldens
src = REPO / "bridge-contract/examples/flows"
for part, n in (("arm_report_goldens", "arms.json"), ("bridge_goldens", "authoring.json"),
                ("bridge_goldens", "writer_evaluation.json")):
    (P / part).mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src / n, P / part / n)
dump("arm_report_goldens/PROVENANCE.json", {
    "source": "bridge-contract/examples/flows/arms.json", "copied_at_main": "853b029",
    "note": "byte copy; ArmReport schema lives in bridge-contract/schemas/ArmReport.schema.json"})
dump("bridge_goldens/PROVENANCE.json", {
    "sources": ["bridge-contract/examples/flows/authoring.json",
                "bridge-contract/examples/flows/writer_evaluation.json"],
    "copied_at_main": "853b029", "note": "byte copies; dry-run is flow authoring, writer is writer_evaluation"})
