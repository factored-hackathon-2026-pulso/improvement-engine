//! Typed arms (`run_arm`, `run_arm_by_id`, `read_arm`, `read_arm_by_key`) against the generated golden FakeCore.
mod common;
use common::golden::{self, GoldenCore};
use common::{FakeCore, KID, SEED};
use core_client::dto::{ArmMode, ArmRequest, ArmStatus, ExecutionProfile};
use core_client::{CallError, ClientConfig, CoreClient, OpError};
use serde_json::json;

fn client(addr: &str) -> CoreClient {
    let mut cfg = ClientConfig::new(addr, KID, SEED, "bridge-1");
    cfg.timeout = std::time::Duration::from_secs(5);
    CoreClient::new(cfg)
}

/// The golden `arm_run_native` request: deprecated alias spelling (`mode`), key and agent_id in the body.
fn native() -> ArmRequest {
    let mut r = ArmRequest::new("golden-arm-1", "bind-arm", "case-1", "baseline", 0, json!(7), json!({"kind":"published_release","release_id":"rel-demo"}), "art-golden", "bud-1");
    r.mode = Some(ArmMode::Native);
    r.agent_id = Some("atencion".into());
    r.campaign_ref = Some("camp-1".into());
    r
}

fn code_of(e: OpError) -> String {
    match e {
        OpError::Call(CallError::Api(a)) => a.code,
        e => panic!("{e:?}"),
    }
}

#[test]
fn run_arm_encodes_the_golden_request_and_decodes_the_report() {
    let g = GoldenCore::play(&[("arms", "arm_run_native")]);
    let rep = client(&g.addr).run_arm("t1", Some("job-golden"), &native()).unwrap();
    g.finish();
    assert_eq!(rep.status, ArmStatus::Completed);
    assert!(rep.is_completed());
    assert_eq!(rep.arm.as_deref(), Some("baseline"));
    assert_eq!(rep.case_ref.as_deref(), Some("case-1"));
    assert_eq!(rep.cost_known, Some(true));
    assert_eq!(rep.usage.as_ref().unwrap()["cost_usd"], "0.002");
    assert_eq!(rep.target_commitment.as_deref(), Some("99a37b63b74a1173e45135e3d89270244c03ed4234ac904197a77c1fcc249c38"));
    // the key rides in the header AND in the body, equal (contract.json idempotency.arms.key)
    let req = &g.requests()[0];
    assert_eq!(req.headers["idempotency-key"], "golden-arm-1");
    let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(body["idempotency_key"], "golden-arm-1");
}

#[test]
fn replay_returns_the_stored_report_and_another_body_conflicts() {
    let g = GoldenCore::play(&[("arms", "arm_run_native"), ("arms", "arm_run_replay"), ("arms", "arm_run_conflict")]);
    let c = client(&g.addr);
    let (a, b) = (c.run_arm("t1", Some("job-golden"), &native()).unwrap(), c.run_arm("t1", Some("job-golden"), &native()).unwrap());
    assert_eq!(a.execution_id, b.execution_id);
    assert_eq!(a.raw, b.raw);
    let mut other = native();
    other.seed = json!(8);
    assert_eq!(code_of(c.run_arm("t1", Some("job-golden"), &other).unwrap_err()), "pulso:idempotency_conflict");
    g.finish();
}

#[test]
fn readback_by_id_and_by_key() {
    let g = GoldenCore::play(&[("arms", "arm_run_native"), ("arms", "arm_read_by_id"), ("arms", "arm_read_by_key")]);
    let c = client(&g.addr);
    let run = c.run_arm("t1", Some("job-golden"), &native()).unwrap();
    let by_id = c.read_arm("t1", Some("job-golden"), &run.execution_id).unwrap();
    let by_key = c.read_arm_by_key("t1", Some("job-golden"), "golden-arm-1").unwrap();
    g.finish();
    assert_eq!(by_id.raw, run.raw);
    assert_eq!(by_key.raw, run.raw);
}

#[test]
fn unknown_key_and_foreign_tenant_are_not_found() {
    let foreign = golden::step("arms", "arm_read_foreign_tenant");
    let g = GoldenCore::play(&[("arms", "arm_read_unknown"), ("arms", "arm_read_foreign_tenant")]);
    let c = client(&g.addr);
    assert_eq!(code_of(c.read_arm_by_key("t1", Some("job-golden"), "never-seen").unwrap_err()), "pulso:not_found");
    let t = foreign.tenant_id.unwrap();
    assert_eq!(code_of(c.read_arm(&t, foreign.job_id.as_deref(), "<trace_id#1>").unwrap_err()), "pulso:not_found");
    g.finish();
}

#[test]
fn closed_refusals_are_terminal_conflicts() {
    let g = GoldenCore::play(&[("arms", "arm_mixed_world"), ("arms", "arm_sandbox_required")]);
    let c = client(&g.addr);
    let mut mixed = native();
    mixed.idempotency_key = "golden-arm-mixed".into();
    mixed.seed_manifest_ref = Some("seed-1".into());
    assert_eq!(code_of(c.run_arm("t1", Some("job-golden"), &mixed).unwrap_err()), "pulso:mixed_world_rejected");
    let mut nosb = native();
    nosb.idempotency_key = "golden-arm-nosb".into();
    nosb.mode = Some(ArmMode::TaskBuilder);
    assert_eq!(code_of(c.run_arm("t1", Some("job-golden"), &nosb).unwrap_err()), "pulso:sandbox_required");
    g.finish();
}

#[test]
fn in_run_failure_is_a_report_not_an_error() {
    let g = GoldenCore::play(&[("arms", "arm_budget_unknown")]);
    let mut r = native();
    r.idempotency_key = "golden-arm-nobud".into();
    r.budget_ref = "bud-missing-xyz".into();
    let rep = client(&g.addr).run_arm("t1", Some("job-golden"), &r).unwrap();
    g.finish();
    assert_eq!(rep.status, ArmStatus::FailedInfra);
    assert!(!rep.is_completed());
    assert_eq!(rep.reason.as_deref(), Some("budget_unknown"));
    assert_eq!(rep.usage, None, "usage=null is 'not available'");
    assert_eq!(rep.cost_known, Some(false), "cost_known=false is not free");
}

#[test]
fn a_minimal_denied_report_decodes() {
    let rep = core_client::dto::ArmReport::from_json(&json!({"execution_id":"arm-707546950d0f793fd8f9bb7fda133396","status":"unknown","reason":"interrupted"})).unwrap();
    assert_eq!((rep.status, rep.reason.as_deref(), rep.arm), (ArmStatus::Unknown, Some("interrupted"), None));
    assert!(core_client::dto::ArmReport::from_json(&json!({"execution_id":"x","status":"weird"})).is_err());
    assert!(core_client::dto::ArmReport::from_json(&json!({"status":"completed"})).is_err());
}

#[test]
fn alias_and_annex_spellings_share_one_single_flight_digest() {
    let alias = native();
    let mut annex = native();
    annex.mode = None;
    annex.execution_profile = Some(ExecutionProfile::EvolutionTask);
    annex.agent_id = None;
    // agent_id is filled before hashing: omitted == sent when it equals the one derived from the target
    assert_eq!(alias.single_flight_digest(None).unwrap(), annex.single_flight_digest(Some("atencion")).unwrap());
    // the deprecated session alias and the annex name are one request
    let mut a = native();
    a.seed_manifest_ref = Some("s1".into());
    let mut b = native();
    b.sandbox_session_ref = Some("s1".into());
    assert_eq!(a.single_flight_digest(None).unwrap(), b.single_flight_digest(None).unwrap());
    // `deadline` is a per-attempt bound, not identity
    let mut d = native();
    d.deadline = Some("2030-01-01T00:00:00Z".into());
    assert_eq!(d.single_flight_digest(None).unwrap(), alias.single_flight_digest(None).unwrap());
    // anything else is identity
    let mut s = native();
    s.seed = json!(8);
    assert_ne!(s.single_flight_digest(None).unwrap(), alias.single_flight_digest(None).unwrap());
    let mut tb = native();
    tb.mode = Some(ArmMode::TaskBuilder);
    assert_ne!(tb.single_flight_digest(None).unwrap(), alias.single_flight_digest(None).unwrap());
}

#[test]
fn single_flight_digest_has_the_python_canonical_shape() {
    // python: ArmRequest.canonical() of the golden request, digest = sha256(canonical JSON, sorted keys)
    let canon = native().single_flight_canonical(None);
    let expect = json!({"schema_version":"1","idempotency_key":"golden-arm-1","binding_ref":"bind-arm","case_ref":"case-1",
        "campaign_ref":"camp-1","arm":"baseline","repetition":0,"seed":7,"target":{"kind":"published_release","release_id":"rel-demo"},
        "target_commitment":null,"scenario_manifest_ref":"art-golden","oracle_ref":null,"budget_ref":"bud-1",
        "supersedes_execution_id":null,"agent_id":"atencion","execution_profile":"evolution_task","sandbox_session_ref":null});
    assert_eq!(canon, expect);
}

#[test]
fn annex_spelling_encodes_execution_profile_and_session() {
    let mut r = native();
    r.mode = None;
    r.agent_id = None;
    r.execution_profile = Some(ExecutionProfile::AttentionStatefulComplementary);
    r.sandbox_session_ref = Some("seed-e2e".into());
    r.deadline = Some("2030-01-01T00:00:00Z".into());
    let b = r.to_json();
    assert_eq!(b["execution_profile"], "attention_stateful_complementary");
    assert_eq!(b["sandbox_session_ref"], "seed-e2e");
    assert_eq!(b["deadline"], "2030-01-01T00:00:00Z");
    assert!(b.get("mode").is_none() && b.get("agent_id").is_none() && b.get("seed_manifest_ref").is_none());
    assert_eq!(b["schema_version"], "1");
}

#[test]
fn invalid_arm_requests_are_refused_before_sending() {
    let f = FakeCore::start();
    let c = client(&f.addr);
    let mut r = native();
    r.mode = None;
    assert!(matches!(c.run_arm("t1", None, &r), Err(OpError::Invalid(_))), "neither profile nor mode");
    let mut r = native();
    r.execution_profile = Some(ExecutionProfile::AttentionStatefulComplementary); // mode native contradicts it
    assert!(matches!(c.run_arm("t1", None, &r), Err(OpError::Invalid(_))));
    let mut r = native();
    r.sandbox_session_ref = Some("a".into());
    r.seed_manifest_ref = Some("b".into());
    assert!(matches!(c.run_arm("t1", None, &r), Err(OpError::Invalid(_))));
    let mut r = native();
    r.deadline = Some("2030-01-01T00:00:00+00:00".into());
    assert!(matches!(c.run_arm("t1", None, &r), Err(OpError::Invalid(_))), "deadline must be Z");
    let mut r = native();
    r.idempotency_key = "a b".into();
    assert!(matches!(c.run_arm("t1", None, &r), Err(OpError::Invalid(_))), "key charset");
    let mut r = native();
    r.idempotency_key = "a|b".into();
    assert!(matches!(c.run_arm("t1", None, &r), Err(OpError::Invalid(_)) | Err(OpError::Canon(_))), "pipe in key");
    let mut r = native();
    r.target = json!([1]);
    assert!(matches!(c.run_arm("t1", None, &r), Err(OpError::Invalid(_))));
    let mut r = native();
    r.seed = json!(1.5);
    assert!(matches!(c.run_arm("t1", None, &r), Err(OpError::Invalid(_))));
    assert!(matches!(c.run_arm("t|1", None, &native()), Err(OpError::Canon(_))), "pipe in tenant");
    assert!(f.requests().is_empty());
}

#[test]
fn a_report_for_another_execution_is_a_contract_violation() {
    let f = FakeCore::start();
    f.script(200, json!({"execution_id":"arm-00000000000000000000000000000000","status":"completed"}));
    assert!(matches!(client(&f.addr).run_arm("t1", None, &native()), Err(OpError::Contract(_))));
}

#[test]
fn the_correct_execution_id_is_accepted() {
    let f = FakeCore::start();
    let id = core_client::canon::arm_execution_id("t1", "golden-arm-1").unwrap();
    f.script(200, json!({"execution_id": id, "status":"completed"}));
    assert!(client(&f.addr).run_arm("t1", None, &native()).unwrap().is_completed());
}

#[test]
fn run_arm_by_id_uses_its_route_and_purpose() {
    let f = FakeCore::start();
    f.script(200, json!({"execution_id":"<x>","status":"completed"}));
    client(&f.addr).run_arm_by_id("t1", Some("job-9"), "arm-abc", &native()).unwrap();
    let r = &f.requests()[0];
    assert_eq!((r.method.as_str(), r.path.as_str()), ("POST", "/internal/v1/evaluation/arms/arm-abc/run"));
    assert_eq!(r.claims.as_ref().unwrap()["purpose"], "evaluation_arm_run");
    assert_eq!(r.headers["idempotency-key"], "golden-arm-1");
}
