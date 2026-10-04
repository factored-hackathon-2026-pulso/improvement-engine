//! V1: "captured N of 6" is COMPUTED from recorded fixtures (`tests/fixtures/v1/<outcome>.json`), never asserted.
//! A fixture counts only if it is complete (provenance), its observation classifies to the outcome its name claims, and
//! the label is honest (a lost result is always `fault-injected`). Synthetic temp-dir fixtures test the rules; the last
//! test reads the repo's real fixtures.
use eval::capture::{CaptureStatus, FIXTURE_DIR, capture_from_dir, capture_six, captured_count};
use eval::outcomes::EvaluateOutcome as O;
use serde_json::{Value, json};
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("v1cap-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn prov(label: &str) -> Value {
    json!({"stack_image": "localhost/pulso-core-runtime:c814c2b-920f5e3", "agent_core_sha": "c814c2bad9f154d10c092326558815dca9562be7",
           "date": "2026-10-04", "command": "cargo test live_capture", "label": label})
}

fn write(dir: &PathBuf, outcome: &str, provenance: Value, observation: Value) {
    std::fs::write(dir.join(format!("{outcome}.json")), serde_json::to_string(&json!({"outcome": outcome, "provenance": provenance, "observation": observation})).unwrap()).unwrap();
}

fn receipt(native: Value, state: &str) -> Value {
    json!({"kind": "stage_receipt", "http_status": 200, "state": "terminal_ok", "outcome": "completed", "write_ops": ["evaluate"], "native_evaluation": native, "proposal_state_after": state})
}

fn status(caps: &[eval::capture::Capture], o: O) -> &CaptureStatus {
    &caps.iter().find(|c| c.outcome == o).unwrap().status
}

#[test]
fn an_empty_fixture_dir_captures_nothing_and_says_why() {
    let caps = capture_from_dir(&tmp("empty"));
    assert_eq!(caps.len(), 6);
    assert_eq!(captured_count(&caps), 0);
    assert!(matches!(status(&caps, O::Fail), CaptureStatus::NotCaptured { reason } if reason.contains("no fixture")));
}

#[test]
fn a_complete_fixture_whose_observation_classifies_to_its_name_is_captured() {
    let d = tmp("ok");
    write(&d, "pass", prov("real"), receipt(json!({"verdict": "pass", "eval_run_ref": "r", "report_digest": "d"}), "evaluated"));
    write(&d, "failed_infra", prov("real"), receipt(json!({"verdict": "failed_infra", "eval_run_ref": "r", "report_digest": "d"}), "candidate"));
    write(&d, "candidate_changed", prov("real"), json!({"kind": "http_error", "http_status": 409, "code": "pulso:candidate_changed", "stage": "admission"}));
    let caps = capture_from_dir(&d);
    assert_eq!(captured_count(&caps), 3);
    assert!(matches!(status(&caps, O::Pass), CaptureStatus::Captured { .. }));
    assert!(matches!(status(&caps, O::QuotaExceeded), CaptureStatus::NotCaptured { .. }));
}

#[test]
fn a_failed_gate_is_recognised_only_when_the_proposal_went_back_to_draft() {
    let d = tmp("fail");
    write(&d, "fail", prov("real"), receipt(Value::Null, "draft"));
    assert!(matches!(status(&capture_from_dir(&d), O::Fail), CaptureStatus::Captured { .. }));
    write(&d, "fail", prov("real"), receipt(Value::Null, "candidate"));
    assert!(matches!(status(&capture_from_dir(&d), O::Fail), CaptureStatus::NotCaptured { reason } if reason.contains("candidate")), "no verdict and still frozen is not a failed gate");
}

#[test]
fn a_fixture_that_classifies_to_another_outcome_is_refused_never_counted() {
    let d = tmp("mislabeled");
    write(&d, "fail", prov("real"), receipt(json!({"verdict": "pass", "eval_run_ref": "r", "report_digest": "d"}), "evaluated"));
    let caps = capture_from_dir(&d);
    assert_eq!(captured_count(&caps), 0);
    assert!(matches!(status(&caps, O::Fail), CaptureStatus::NotCaptured { reason } if reason.contains("pass")));
}

#[test]
fn provenance_is_mandatory_field_by_field() {
    for field in ["stack_image", "agent_core_sha", "date", "command", "label"] {
        let d = tmp(&format!("prov-{field}"));
        let mut p = prov("real");
        p.as_object_mut().unwrap().remove(field);
        write(&d, "pass", p, receipt(json!({"verdict": "pass"}), "evaluated"));
        let caps = capture_from_dir(&d);
        assert!(matches!(status(&caps, O::Pass), CaptureStatus::NotCaptured { reason } if reason.contains(field)), "{field}");
    }
    let d = tmp("prov-label");
    write(&d, "pass", prov("invented"), receipt(json!({"verdict": "pass"}), "evaluated"));
    assert_eq!(captured_count(&capture_from_dir(&d)), 0, "the label is real or fault-injected");
}

#[test]
fn a_lost_result_is_always_fault_injected() {
    let d = tmp("lost");
    let obs = json!({"kind": "timeout", "request_sent": true, "proposal_state_after": "evaluated"});
    write(&d, "evaluation_result_lost", prov("real"), obs.clone());
    assert_eq!(captured_count(&capture_from_dir(&d)), 0, "a timeout we caused is not a real one");
    write(&d, "evaluation_result_lost", prov("fault-injected"), obs);
    let caps = capture_from_dir(&d);
    assert!(matches!(status(&caps, O::ResultLost), CaptureStatus::Captured { source } if source.contains("fault-injected")));
}

#[test]
fn a_timeout_where_the_request_never_left_is_not_a_lost_result() {
    let d = tmp("unsent");
    write(&d, "evaluation_result_lost", prov("fault-injected"), json!({"kind": "timeout", "request_sent": false, "proposal_state_after": null}));
    assert_eq!(captured_count(&capture_from_dir(&d)), 0);
}

#[test]
fn the_recorded_fixtures_of_the_repo_are_all_valid_and_the_count_matches_the_files() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR);
    let files: Vec<String> = std::fs::read_dir(&dir).map(|r| r.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".json")).collect()).unwrap_or_default();
    let caps = capture_six();
    assert_eq!(captured_count(&caps), files.len(), "every fixture file must be a valid capture (and nothing else counts): {files:?}");
    eprintln!("V1 captured {} of 6 (computed from {} fixtures)", captured_count(&caps), files.len());
}
