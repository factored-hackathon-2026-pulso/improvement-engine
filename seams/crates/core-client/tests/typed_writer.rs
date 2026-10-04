//! K3: draft plan -> dry-run -> sealed commitment -> writer-stage invoke -> readback, against the FakeCore.
//! The writer stage result must EQUAL the sealed commitment: every difference is `OpError::CommitmentMismatch`.
mod common;
use common::{FakeCore, KID, SEED};
use core_client::authoring::{Alias, AliasState, Change, DryRunRequest};
use core_client::canon;
use core_client::writer::{ArtifactSealer, DraftPlan, WriteCommitment, WriterRun, check_alias_readback};
use core_client::{ClientConfig, CoreClient, OpError};
use serde_json::{Value, json};
use std::sync::Mutex;

const BASE: &str = "rel-demo";
const DRY_HASH: &str = "11111111111111111111111111111111111111111111111111111111111111aa";

fn client(addr: &str) -> CoreClient {
    let mut cfg = ClientConfig::new(addr, KID, SEED, "bridge-1");
    cfg.timeout = std::time::Duration::from_secs(5);
    CoreClient::new(cfg)
}

fn fixture() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/k3_draft.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn plan() -> DraftPlan {
    let fx = fixture();
    let changes = fx["plan"]["changes"].as_array().unwrap().iter().map(|c| Change::new(c["kind"].as_str().unwrap(), c["content"].clone(), c["docs"].clone())).collect();
    DraftPlan::new(fx["plan"]["agent_id"].as_str().unwrap(), fx["plan"]["title"].as_str().unwrap(), changes)
}

fn run() -> WriterRun {
    WriterRun {
        tenant_id: "t1".into(),
        job_id: "job-k3".into(),
        logical_key: "k3-writer-1".into(),
        pulso_run_ref: "pr-job-k3".into(),
        lab_grant_ref: "grant-contract".into(),
        writer_release_id: "rel-writer".into(),
        writer_agent_version: "1.0.0".into(),
    }
}

#[derive(Default)]
struct Sealer(Mutex<Vec<(String, Value)>>);
impl ArtifactSealer for Sealer {
    fn seal(&self, plan_ref: &str, content: &Value) -> Result<(), String> {
        self.0.lock().unwrap().push((plan_ref.into(), content.clone()));
        Ok(())
    }
}

fn receipts(ops: &[&str]) -> Vec<Value> {
    ops.iter().enumerate().map(|(i, op)| json!({"key_digest": format!("kd{i}"), "op": op, "request_hash": format!("rh{i}"), "rev_after": i.min(1), "verified": true})).collect()
}

/// Scripts the dry-run answer, then the golden writer answer patched with `patch` over the receipts fact value.
fn script_happy(f: &FakeCore, patch: impl FnOnce(&mut Value)) {
    let p = plan();
    let req = DryRunRequest::new("t1", &p.agent_id, Some(BASE), p.changes.clone());
    f.script(200, json!({"schema_version":"1","valid":true,"violations":[],"candidate_hash":format!("sha256:{DRY_HASH}"),
        "release_id_preview":"rel-prev","request_digest":canon::request_digest(&req.to_json()).unwrap(),"proposal_created":false,"content_hashes":{}}));
    let key = canon::idempotency_key("t1", "job-k3", "writer", 1, "k3-writer-1").unwrap();
    let binding = canon::task_binding_ref("t1", &key).unwrap();
    let mut body = common::golden::golden_response("writer_evaluation", "writer_create_put_freeze");
    body["task_binding_ref"] = json!(binding);
    body["receipt"]["task_binding_ref"] = json!(binding);
    let v = &mut body["result"]["facts"]["pulso_writer_receipts"]["value"];
    *v = json!({"candidate_hash": DRY_HASH, "native_evaluation": null, "proposal_id": "prop-1", "rev": 1, "schema_version": "1",
        "state": "confirmed", "write_receipts": receipts(&["create_proposal", "put_draft", "freeze"])});
    patch(v);
    f.script(200, body);
}

fn freeze(f: &FakeCore, s: &Sealer) -> Result<core_client::writer::FrozenProposal, OpError> {
    client(&f.addr).freeze_draft(s, &plan(), &run(), Some(BASE))
}

// ---- Python parity of the digests --------------------------------------------------------------------------------

#[test]
fn draft_digests_equal_the_python_references() {
    let fx = fixture();
    assert_eq!(plan().put_draft_digest().unwrap(), fx["put_draft_digest"].as_str().unwrap());
    assert_eq!(plan().artifact_digest().unwrap(), fx["artifact_digest"].as_str().unwrap());
}

#[test]
fn the_write_commitment_has_the_golden_wire_shape() {
    let c = WriteCommitment::for_plan(&plan(), Some(BASE)).unwrap();
    assert_eq!(c.to_json(), json!({"base_release_id":BASE,"create_agent_id":"atencion","create_origin":"builder_chat",
        "create_title":"pulso-key:k3fixture","mode":"write","operations":["create_proposal","put_draft","freeze"],
        "put_draft_digest":fixture()["put_draft_digest"]}));
}

// ---- the happy path ----------------------------------------------------------------------------------------------

#[test]
fn freeze_draft_dry_runs_seals_commits_and_returns_the_proposal() {
    let f = FakeCore::start();
    script_happy(&f, |_| {});
    let s = Sealer::default();
    let fz = freeze(&f, &s).unwrap();
    assert_eq!((fz.proposal_id.as_str(), fz.candidate_hash.as_str()), ("prop-1", DRY_HASH));
    assert_eq!(fz.release_id_preview.as_deref(), Some("rel-prev"));
    let sealed = s.0.lock().unwrap();
    assert_eq!(sealed.len(), 1);
    assert_eq!((sealed[0].0.as_str(), &sealed[0].1), (fz.plan_ref.as_str(), &plan().artifact()));
    let reqs = f.requests();
    assert_eq!(reqs[0].path, "/internal/v1/core-authoring/dry-run");
    assert_eq!(reqs[1].path, "/internal/v1/core-tasks/invoke");
    let body: Value = serde_json::from_slice(&reqs[1].body).unwrap();
    assert_eq!(body["registry_mutation_commitment"], fz.commitment.to_json());
    assert_eq!(body["input"], json!({"base_release_id":BASE,"draft_plan_ref":fz.plan_ref,"evaluate_enabled":false,"proposal_id":null}));
    assert_eq!((body["agent_id"].as_str(), body["stage"].as_str()), (Some("pulso-writer"), Some("writer")));
}

// ---- commitment mismatch: the first RED -------------------------------------------------------------------------

fn assert_mismatch(patch: impl FnOnce(&mut Value), why: &str) {
    let f = FakeCore::start();
    script_happy(&f, patch);
    match freeze(&f, &Sealer::default()) {
        Err(OpError::CommitmentMismatch(m)) => assert!(!m.is_empty(), "{why}"),
        other => panic!("{why}: expected CommitmentMismatch, got {other:?}"),
    }
}

#[test]
fn any_difference_between_the_stage_result_and_the_sealed_commitment_is_an_error() {
    assert_mismatch(|v| v["write_receipts"] = json!(receipts(&["create_proposal", "put_draft"])), "freeze missing");
    assert_mismatch(|v| v["write_receipts"] = json!(receipts(&["create_proposal", "put_draft", "freeze", "freeze"])), "extra op");
    assert_mismatch(|v| v["write_receipts"] = json!(receipts(&["create_proposal", "freeze", "put_draft"])), "order");
    assert_mismatch(|v| v["write_receipts"] = json!(receipts(&["create_proposal", "put_draft", "evaluate"])), "wrong op");
    assert_mismatch(|v| v["write_receipts"][1]["verified"] = json!(false), "unverified write");
    assert_mismatch(|v| v["candidate_hash"] = json!("22222222222222222222222222222222222222222222222222222222222222bb"), "other candidate");
    assert_mismatch(|v| v["candidate_hash"] = Value::Null, "no candidate");
    assert_mismatch(|v| v["state"] = json!("manual_reconcile"), "state");
    assert_mismatch(|v| v["proposal_id"] = Value::Null, "no proposal");
    assert_mismatch(|v| v["native_evaluation"] = json!({"verdict":"pass"}), "an evaluation nobody committed");
}

#[test]
fn a_golden_placeholder_hash_is_a_mismatch_for_a_strict_client() {
    assert_mismatch(|v| v["candidate_hash"] = json!("<candidate_hash#1>"), "placeholder");
}

// ---- guards before anything is sent -----------------------------------------------------------------------------

#[test]
fn a_non_integer_number_in_a_draft_is_refused_before_any_request() {
    let f = FakeCore::start();
    let mut p = plan();
    p.changes[1].content["seed"] = json!(120.5);
    let e = client(&f.addr).freeze_draft(&Sealer::default(), &p, &run(), Some(BASE)).unwrap_err();
    assert!(matches!(e, OpError::Invalid(_)), "{e:?}");
    assert!(f.requests().is_empty());
}

#[test]
fn a_refused_dry_run_stops_before_sealing_or_writing() {
    let f = FakeCore::start();
    f.script(200, json!({"schema_version":"1","valid":false,"violations":[{"rule":"r","message":"m"}],"candidate_hash":null}));
    let s = Sealer::default();
    assert!(matches!(freeze(&f, &s), Err(OpError::DryRunRefused(_))));
    assert!(s.0.lock().unwrap().is_empty());
    assert_eq!(f.requests().len(), 1);
}

// ---- readback -----------------------------------------------------------------------------------------------------

fn alias(release: Option<&str>, which: Alias) -> AliasState {
    AliasState::from_json(&json!({"schema_version":"1","agent_id":"atencion","alias":which.as_str(),"release_id":release})).unwrap()
}

#[test]
fn alias_readback_must_show_exactly_the_published_release() {
    assert!(check_alias_readback(&alias(Some("rel-new"), Alias::Staging), "rel-new", Some(BASE)).is_ok());
    for (a, why) in [
        (alias(Some("rel-other"), Alias::Staging), "other release"),
        (alias(None, Alias::Staging), "no release"),
        (alias(Some("rel-new"), Alias::Prod), "prod must be untouched by a staging publish"),
    ] {
        assert!(matches!(check_alias_readback(&a, "rel-new", Some(BASE)), Err(OpError::CommitmentMismatch(_))), "{why}");
    }
    assert!(matches!(check_alias_readback(&alias(Some(BASE), Alias::Staging), BASE, Some(BASE)), Err(OpError::CommitmentMismatch(_))), "publishing the base is no publish");
}

#[test]
fn the_evaluate_only_commitment_and_writer_input_have_the_golden_wire_shape() {
    use core_client::writer::{EvaluateOnlyCommitment, WriterInput};
    let g: Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../contracts/engine-steps/pack/parts/bridge_goldens/writer_evaluation.json")).unwrap()).unwrap();
    let c = EvaluateOnlyCommitment { base_release_id: Some("rel-demo".into()), proposal_id: "P1".into(), evaluation_context_ref: "evc-1".into() }.to_json();
    assert_eq!(c, json!({"base_release_id":"rel-demo","evaluate_enabled":true,"evaluation_context_ref":"evc-1","mode":"evaluate_only","operations":[],"proposal_id":"P1"}));
    let i = WriterInput { draft_plan_ref: "plan-golden".into(), base_release_id: Some("rel-demo".into()), evaluate_enabled: true, proposal_id: Some("P1".into()), evaluation_suite: Some(("disputas-suite".into(), "1.0.0".into())) }.to_json();
    let s = g.to_string();
    assert!(s.contains("\"evaluation_suite_id\":\"disputas-suite\""), "golden moved");
    assert_eq!(i, json!({"base_release_id":"rel-demo","draft_plan_ref":"plan-golden","evaluate_enabled":true,"evaluation_suite_id":"disputas-suite","evaluation_suite_version":"1.0.0","proposal_id":"P1"}));
}
