//! Typed evaluation admissions against the generated golden FakeCore (writer_evaluation flow).
mod common;
use common::golden::GoldenCore;
use common::{FakeCore, KID, SEED};
use core_client::dto::{AdmissionRequest, AdmissionState};
use core_client::{CallError, ClientConfig, CoreClient, OpError};
use serde_json::json;

const JOB: &str = "job-golden-eval";
const BINDING: &str = "f5f19be72642cfcff1ff9ca604402acb7769f67306f25ab257e981945c61ee33";

fn client(addr: &str) -> CoreClient {
    let mut cfg = ClientConfig::new(addr, KID, SEED, "bridge-1");
    cfg.timeout = std::time::Duration::from_secs(5);
    CoreClient::new(cfg)
}

fn adm() -> AdmissionRequest {
    AdmissionRequest::new(
        BINDING,
        "<proposal_id#1>",
        "<candidate_hash#1>",
        "disputas-suite",
        "1.0.0",
        "13a1031b0f08ebe5285f74fd07c4dc1ed94e85d47c08bbd576d373050b62b5a7",
        1,
        "bud-1",
        "2030-01-01T00:00:00Z",
    )
}

fn code_of(e: OpError) -> String {
    match e {
        OpError::Call(CallError::Api(a)) => a.code,
        e => panic!("{e:?}"),
    }
}

#[test]
fn first_admission_is_201_and_a_replay_is_200_with_the_same_ref() {
    let g = GoldenCore::play(&[("writer_evaluation", "admission_created"), ("writer_evaluation", "admission_replay")]);
    let c = client(&g.addr);
    let first = c.admit_evaluation("t1", JOB, &adm()).unwrap();
    // a retry recomputes the deadline: it is not identity, so the request digest is unchanged
    let mut later = adm();
    later.deadline = "2031-06-30T12:00:00Z".into();
    let replay = c.admit_evaluation("t1", JOB, &later).unwrap();
    g.finish();
    assert!(first.created && !replay.created);
    assert_eq!(first.admission.state, AdmissionState::Admitted);
    assert_eq!(first.admission.evaluation_context_ref, replay.admission.evaluation_context_ref);
    let reqs = g.requests();
    assert!(reqs[0].headers.get("idempotency-key").is_none(), "the derived ref identifies an admission; the header is optional");
    let (b0, b1): (serde_json::Value, serde_json::Value) = (serde_json::from_slice(&reqs[0].body).unwrap(), serde_json::from_slice(&reqs[1].body).unwrap());
    assert_eq!(b0["request_digest"], b1["request_digest"]);
    assert_ne!(b0["deadline"], b1["deadline"]);
    assert!(b0.get("evaluation_context_ref").is_none(), "deprecated: the bridge derives it");
}

#[test]
fn request_digest_is_jcs_of_the_body_without_deadline() {
    let r = adm();
    let body = r.to_json();
    let mut m = body.as_object().unwrap().clone();
    m.remove("deadline");
    m.remove("request_digest");
    let expect = core_client::canon::digest_json(&serde_json::Value::Object(m)).unwrap();
    assert_eq!(body["request_digest"], expect);
    assert_eq!(body["evaluation_attempt"], 1);
    assert_eq!(body["schema_version"], "1");
    let mut other = adm();
    other.budget_ref = "bud-2".into();
    assert_ne!(other.to_json()["request_digest"], body["request_digest"]);
}

#[test]
fn derived_context_ref_follows_the_contract_formula() {
    let r = adm().derived_context_ref("t1", JOB).unwrap();
    assert_eq!(r, core_client::canon::evaluation_context_ref("t1", JOB, BINDING, "<proposal_id#1>", "<candidate_hash#1>", 1).unwrap());
    assert!(r.starts_with("evc-") && r.len() == 44);
}

#[test]
fn a_digest_not_bound_to_the_fields_conflicts() {
    let g = GoldenCore::play(&[("writer_evaluation", "admission_created"), ("writer_evaluation", "admission_conflict")]);
    let c = client(&g.addr);
    c.admit_evaluation("t1", JOB, &adm()).unwrap();
    let mut forged = adm();
    forged.request_digest_override = Some("1".repeat(64));
    assert_eq!(code_of(c.admit_evaluation("t1", JOB, &forged).unwrap_err()), "pulso:idempotency_conflict");
    g.finish();
}

#[test]
fn stale_candidate_is_a_terminal_conflict() {
    let g = GoldenCore::play(&[("writer_evaluation", "admission_stale_candidate")]);
    let mut stale = adm();
    stale.candidate_hash = "<candidate_hash#2>".into();
    assert_eq!(code_of(client(&g.addr).admit_evaluation("t1", JOB, &stale).unwrap_err()), "pulso:candidate_changed");
    g.finish();
}

#[test]
fn invalid_admissions_are_refused_before_sending() {
    let f = FakeCore::start();
    let c = client(&f.addr);
    let mut r = adm();
    r.deadline = "2030-01-01T00:00:00+00:00".into();
    assert!(matches!(c.admit_evaluation("t1", JOB, &r), Err(OpError::Invalid(_))), "deadline must be Z");
    let mut r = adm();
    r.evaluation_attempt = 0;
    assert!(matches!(c.admit_evaluation("t1", JOB, &r), Err(OpError::Invalid(_))));
    let mut r = adm();
    r.suite_id = String::new();
    assert!(matches!(c.admit_evaluation("t1", JOB, &r), Err(OpError::Invalid(_))));
    let mut r = adm();
    r.request_digest_override = Some("XYZ".into());
    assert!(matches!(c.admit_evaluation("t1", JOB, &r), Err(OpError::Invalid(_))));
    let mut r = adm();
    r.proposal_id = "p|q".into();
    assert!(matches!(c.admit_evaluation("t1", JOB, &r), Err(OpError::Canon(_))), "`|` is rejected in the derivation");
    assert!(matches!(c.admit_evaluation("t1", "j|b", &adm()), Err(OpError::Canon(_))));
    assert!(f.requests().is_empty());
}

#[test]
fn a_ref_that_is_not_the_derived_one_is_a_contract_violation() {
    let f = FakeCore::start();
    f.script(201, json!({"schema_version":"1","evaluation_context_ref":"evc-0000000000000000000000000000000000000000","state":"admitted"}));
    assert!(matches!(client(&f.addr).admit_evaluation("t1", JOB, &adm()), Err(OpError::Contract(_))));
}

#[test]
fn the_derived_ref_is_accepted_and_states_decode() {
    let f = FakeCore::start();
    let r = adm().derived_context_ref("t1", JOB).unwrap();
    f.script(200, json!({"schema_version":"1","evaluation_context_ref":r,"state":"consumed"}));
    let out = client(&f.addr).admit_evaluation("t1", JOB, &adm()).unwrap();
    assert_eq!((out.created, out.admission.state), (false, AdmissionState::Consumed));
    f.script(200, json!({"schema_version":"1","evaluation_context_ref":r,"state":"weird"}));
    assert!(matches!(client(&f.addr).admit_evaluation("t1", JOB, &adm()), Err(OpError::Decode(_))));
}
