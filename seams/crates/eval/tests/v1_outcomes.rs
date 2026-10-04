use core_client::dto::ArmReport;
use eval::outcomes::{ArmRollup, EvaluateOutcome as O, classify_http, classify_timeout, rollup_arms};
use eval::capture::{CaptureStatus, capture_six};
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
fn capture_from_recorded_goldens_is_honest_about_what_has_no_real_report() {
    let caps = capture_six();
    assert_eq!(caps.len(), 6);
    let get = |o: O| caps.iter().find(|c| c.outcome == o).unwrap();
    assert!(matches!(get(O::Pass).status, CaptureStatus::Captured { .. }));
    assert!(matches!(get(O::FailedInfra).status, CaptureStatus::Captured { .. }));
    for o in [O::Fail, O::QuotaExceeded, O::CandidateChanged, O::ResultLost] {
        assert!(matches!(get(o).status, CaptureStatus::NotCaptured { .. }), "{o:?} must not be fabricated");
    }
    assert_eq!(eval::capture::captured_count(&caps), 2);
}
