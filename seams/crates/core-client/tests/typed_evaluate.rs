//! K3: native evaluation of a frozen proposal (admission + evaluate-only writer stage), against the FakeCore.
//! The registry approves only an `evaluated` proposal, so this driver is the missing step between freeze and approve.
mod common;
use common::{FakeCore, KID, SEED};
use core_client::authoring::Change;
use core_client::canon;
use core_client::evaluate::{BindingPreauthorizer, EvaluationRun, SuiteRef};
use core_client::writer::{DraftPlan, FrozenProposal, WriteCommitment};
use core_client::{ClientConfig, CoreClient, OpError};
use serde_json::{Value, json};
use std::sync::Mutex;

const BASE: &str = "rel-demo";
const CAND: &str = "11111111111111111111111111111111111111111111111111111111111111aa";

fn client(addr: &str) -> CoreClient {
    let mut cfg = ClientConfig::new(addr, KID, SEED, "bridge-1");
    cfg.timeout = std::time::Duration::from_secs(5);
    CoreClient::new(cfg)
}

fn frozen() -> FrozenProposal {
    let plan = DraftPlan::new("atencion", "pulso-key:t", vec![Change::new("prompt", json!({"id": "p/x", "version": "2.0.0"}), json!({}))]);
    FrozenProposal {
        proposal_id: "prop-1".into(),
        candidate_hash: CAND.into(),
        base_release_id: Some(BASE.into()),
        task_binding_ref: "x".into(),
        plan_ref: "plan-1".into(),
        title: "pulso-key:t".into(),
        release_id_preview: None,
        commitment: WriteCommitment::for_plan(&plan, Some(BASE)).unwrap(),
    }
}

fn run() -> EvaluationRun {
    EvaluationRun {
        tenant_id: "t1".into(),
        job_id: "job-eval".into(),
        logical_key: "evalonly".into(),
        attempt: 1,
        pulso_run_ref: "pr-job-eval".into(),
        lab_grant_ref: "grant-contract".into(),
        writer_release_id: "rel-writer".into(),
        writer_agent_version: "1.0.0".into(),
        budget_ref: "bud-1".into(),
        deadline: "2030-01-01T00:00:00Z".into(),
    }
}

fn suite() -> SuiteRef {
    SuiteRef { id: "suite-1".into(), version: "2.0.0".into(), digest: "22".repeat(32) }
}

#[derive(Default)]
struct Pre(Mutex<Vec<(String, String)>>, bool);
impl BindingPreauthorizer for Pre {
    fn preauthorize(&self, tenant: &str, binding_ref: &str) -> Result<(), String> {
        if self.1 {
            return Err("double down".into());
        }
        self.0.lock().unwrap().push((tenant.into(), binding_ref.into()));
        Ok(())
    }
}

fn key() -> String {
    canon::idempotency_key("t1", "job-eval", "writer", 1, "evalonly").unwrap()
}

fn script_admission(f: &FakeCore, ctx: &str) {
    f.script(201, json!({"schema_version": "1", "evaluation_context_ref": ctx, "state": "admitted"}));
}

fn ctx_ref(binding: &str) -> String {
    canon::evaluation_context_ref("t1", "job-eval", binding, "prop-1", CAND, 1).unwrap()
}

fn script_invoke(f: &FakeCore, ops: &[&str], native: Value, state: &str, outcome: &str) {
    let binding = canon::task_binding_ref("t1", &key()).unwrap();
    let mut body = common::golden::golden_response("writer_evaluation", "invoke_evaluate_only");
    body["task_binding_ref"] = json!(binding);
    body["receipt"]["task_binding_ref"] = json!(binding);
    body["state"] = json!(state);
    body["outcome"] = json!(outcome);
    body["result"]["outcome"] = json!(outcome);
    let wr: Vec<Value> = ops.iter().map(|o| json!({"key_digest": "k", "op": o, "request_hash": "r", "rev_after": 1, "verified": true})).collect();
    body["result"]["facts"]["pulso_writer_receipts"]["value"] =
        json!({"candidate_hash": CAND, "native_evaluation": native, "proposal_id": "prop-1", "rev": 1, "schema_version": "1", "state": "confirmed", "write_receipts": wr});
    f.script(200, body);
}

#[test]
fn a_passing_native_evaluation_is_driven_through_binding_admission_and_evaluate_only() {
    let f = FakeCore::start();
    let binding = canon::task_binding_ref("t1", &key()).unwrap();
    script_admission(&f, &ctx_ref(&binding));
    script_invoke(&f, &["evaluate"], json!({"eval_run_ref": "er-1", "report_digest": "d".repeat(64), "verdict": "pass"}), "terminal_ok", "completed");
    let pre = Pre::default();
    let ev = client(&f.addr).evaluate_frozen(&pre, &frozen(), &suite(), &run()).expect("evaluate");
    assert_eq!(ev.verdict(), Some("pass"));
    assert_eq!(ev.binding_ref, binding);
    assert_eq!(pre.0.lock().unwrap().as_slice(), &[("t1".to_string(), binding.clone())], "the binding is pre-authorised before any Core call");
    let reqs = f.requests();
    let paths: Vec<&str> = reqs.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["/internal/v1/evaluation/admissions", "/internal/v1/core-tasks/invoke"]);
    let adm: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!((adm["candidate_hash"].as_str(), adm["proposal_id"].as_str(), adm["binding_ref"].as_str()), (Some(CAND), Some("prop-1"), Some(binding.as_str())));
    let inv: Value = serde_json::from_slice(&reqs[1].body).unwrap();
    let c = &inv["registry_mutation_commitment"];
    assert_eq!(c["mode"], "evaluate_only");
    assert_eq!(c["evaluation_context_ref"], json!(ctx_ref(&binding)));
    assert_eq!((c["operations"].clone(), c["proposal_id"].clone()), (json!([]), json!("prop-1")));
    assert_eq!(inv["input"]["evaluation_suite_id"], "suite-1");
    assert_eq!(inv["input"]["evaluate_enabled"], true);
}

#[test]
fn a_stage_that_wrote_anything_but_the_evaluation_is_a_commitment_mismatch() {
    let f = FakeCore::start();
    let binding = canon::task_binding_ref("t1", &key()).unwrap();
    script_admission(&f, &ctx_ref(&binding));
    script_invoke(&f, &["put_draft", "evaluate"], json!({"eval_run_ref": "er-1", "report_digest": "d".repeat(64), "verdict": "pass"}), "terminal_ok", "completed");
    let r = client(&f.addr).evaluate_frozen(&Pre::default(), &frozen(), &suite(), &run());
    assert!(matches!(r, Err(OpError::CommitmentMismatch(_))), "{r:?}");
}

#[test]
fn a_completed_stage_without_any_evaluate_write_is_a_commitment_mismatch() {
    let f = FakeCore::start();
    let binding = canon::task_binding_ref("t1", &key()).unwrap();
    script_admission(&f, &ctx_ref(&binding));
    script_invoke(&f, &[], Value::Null, "terminal_ok", "completed");
    let r = client(&f.addr).evaluate_frozen(&Pre::default(), &frozen(), &suite(), &run());
    assert!(matches!(r, Err(OpError::CommitmentMismatch(_))), "{r:?}");
}

/// Observed on the real image (pin c814c2b): a failed gate (409 gate_failed inside Core's registry tool) does not
/// surface as an HTTP error of the bridge: the stage completes, the verified `evaluate` write is there, and
/// `native_evaluation` is null (the proposal went back to draft). The driver reports it; classifying it as a gate
/// failure needs the proposal state (`eval::outcomes::classify_evaluation`).
#[test]
fn a_verified_evaluate_write_without_a_native_verdict_is_reported_as_no_verdict() {
    let f = FakeCore::start();
    let binding = canon::task_binding_ref("t1", &key()).unwrap();
    script_admission(&f, &ctx_ref(&binding));
    script_invoke(&f, &["evaluate"], Value::Null, "terminal_ok", "completed");
    let ev = client(&f.addr).evaluate_frozen(&Pre::default(), &frozen(), &suite(), &run()).expect("a completed stage is a result");
    assert_eq!(ev.verdict(), None);
    assert!(ev.receipt.is_success());
}

#[test]
fn a_failed_stage_is_reported_not_raised_and_has_no_native_verdict() {
    let f = FakeCore::start();
    let binding = canon::task_binding_ref("t1", &key()).unwrap();
    script_admission(&f, &ctx_ref(&binding));
    script_invoke(&f, &[], Value::Null, "terminal_failed", "pulso:gate_failed");
    let ev = client(&f.addr).evaluate_frozen(&Pre::default(), &frozen(), &suite(), &run()).expect("a failed run is a result");
    assert_eq!(ev.verdict(), None);
    assert!(!ev.receipt.is_success());
}

#[test]
fn nothing_reaches_core_when_the_binding_cannot_be_pre_authorised() {
    let f = FakeCore::start();
    let r = client(&f.addr).evaluate_frozen(&Pre(Mutex::default(), true), &frozen(), &suite(), &run());
    assert!(matches!(r, Err(OpError::Invalid(_))), "{r:?}");
    assert!(f.requests().is_empty());
}

#[test]
fn an_admission_for_another_context_is_a_contract_error_and_no_stage_is_invoked() {
    let f = FakeCore::start();
    script_admission(&f, "evc-0000000000000000000000000000000000000000");
    let r = client(&f.addr).evaluate_frozen(&Pre::default(), &frozen(), &suite(), &run());
    assert!(matches!(r, Err(OpError::Contract(_))), "{r:?}");
    assert_eq!(f.requests().len(), 1);
}

/// After a lost answer the admission is already consumed and the proposal is no longer frozen, so a second admission
/// would be refused (`candidate_changed`, seen live). The recovery is the IDENTICAL invoke under the same key: Core
/// returns the stored run, no second evaluation.
#[test]
fn a_lost_evaluation_is_recovered_by_the_identical_invoke_without_a_second_admission() {
    let f = FakeCore::start();
    script_invoke(&f, &["evaluate"], json!({"eval_run_ref": "er-1", "report_digest": "d".repeat(64), "verdict": "pass"}), "terminal_ok", "completed");
    let ev = client(&f.addr).replay_evaluation(&frozen(), &suite(), &run()).expect("replay");
    assert_eq!(ev.verdict(), Some("pass"));
    let reqs = f.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path, "/internal/v1/core-tasks/invoke");
    assert_eq!(reqs[0].headers.get("idempotency-key").map(String::as_str), Some(key().as_str()));
}
