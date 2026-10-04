use core_client::dto::ArmReport;
use eval::outcomes::{ArmRollup, EvaluateOutcome as O, classify_http, classify_timeout, rollup_arms};
use serde_json::json;

#[test]
fn http_signals_map_to_outcomes() {
    let rep = |v: &str| json!({"verdict": v});
    assert_eq!(classify_http(200, &rep("pass")), Ok(O::Pass));
    assert_eq!(classify_http(200, &rep("failed_infra")), Ok(O::FailedInfra));
    assert_eq!(classify_http(409, &json!({"code": "gate_failed", "payload": rep("fail")})), Ok(O::Fail));
    assert_eq!(classify_http(409, &json!({"code": "pulso:candidate_changed"})), Ok(O::CandidateChanged));
    assert_eq!(classify_http(429, &json!({"code": "quota_exceeded"})), Ok(O::QuotaExceeded));
    assert_eq!(classify_timeout(), O::ResultLost);
    assert!(classify_http(200, &rep("maybe")).is_err());
    assert!(classify_http(500, &json!({})).is_err());
}

#[test]
fn completed_arms_alone_never_raise_a_verdict() {
    let done = |st: &str| ArmReport::from_json(&json!({"execution_id": "arm-00000000000000000000000000000000", "status": st})).unwrap();
    assert_eq!(rollup_arms(&[done("completed"), done("completed")]), ArmRollup::NoVerdict);
    assert_eq!(rollup_arms(&[done("completed"), done("failed_infra")]), ArmRollup::Outcome(O::FailedInfra));
    assert_eq!(rollup_arms(&[done("candidate_failed"), done("completed")]), ArmRollup::Outcome(O::Fail));
    assert_eq!(rollup_arms(&[done("unknown")]), ArmRollup::Outcome(O::ResultLost));
    assert_eq!(rollup_arms(&[]), ArmRollup::NoVerdict);
}

#[test]
fn the_contract_goldens_are_references_not_captures() {
    // The bridge-contract goldens carry a synthetic pass and a failed_infra ARM report. They document the shape, they are
    // not real evaluate-level captures and never count towards "captured N of 6" (see v1_capture.rs).
    let refs = eval::capture::golden_references();
    assert_eq!(refs.len(), 2);
    assert!(refs.iter().all(|r| r.source.contains("bridge-contract/examples/flows")));
}

#[test]
fn observations_classify_like_the_http_signals() {
    use eval::outcomes::classify_observation;
    let rec = |native: serde_json::Value, state: &str| json!({"kind": "stage_receipt", "http_status": 200, "state": "terminal_ok", "outcome": "completed", "write_ops": ["evaluate"], "native_evaluation": native, "proposal_state_after": state});
    assert_eq!(classify_observation(&rec(json!({"verdict": "pass"}), "evaluated")), Ok(O::Pass));
    assert_eq!(classify_observation(&rec(json!({"verdict": "failed_infra"}), "candidate")), Ok(O::FailedInfra));
    assert_eq!(classify_observation(&rec(json!(null), "draft")), Ok(O::Fail));
    assert!(classify_observation(&rec(json!(null), "candidate")).is_err());
    assert_eq!(classify_observation(&json!({"kind": "http_error", "http_status": 429, "code": "quota_exceeded"})), Ok(O::QuotaExceeded));
    assert_eq!(classify_observation(&json!({"kind": "http_error", "http_status": 409, "code": "pulso:candidate_changed"})), Ok(O::CandidateChanged));
    assert_eq!(classify_observation(&json!({"kind": "timeout", "request_sent": true})), Ok(O::ResultLost));
    assert!(classify_observation(&json!({"kind": "timeout", "request_sent": false})).is_err());
    assert!(classify_observation(&json!({"kind": "weird"})).is_err());
}
