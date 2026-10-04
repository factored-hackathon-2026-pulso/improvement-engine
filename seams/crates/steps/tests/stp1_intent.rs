mod stp1_common;
use std::sync::Mutex;
use steps::{StepError, intent};
use stp1_common::*;

static ENV: Mutex<()> = Mutex::new(());

fn run_with(recompute_out: Option<&str>, input: &str) -> Result<String, StepError> {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("stp1-int-{}-{}", std::process::id(), recompute_out.map_or(0, |s| s.len())));
    std::fs::create_dir_all(&dir).unwrap();
    if let Some(r) = recompute_out {
        std::fs::write(dir.join("sample-1.json"), r).unwrap();
    }
    unsafe { std::env::set_var("STEPS_RECOMPUTE_DIR", &dir) };
    intent::run(input)
}

const MATCH: &str = r#"{"recomputes":[{"signal_id":"sig-0001","claimed_rate":0.3,"recomputed_rate":0.3,"match":true,"evidence_ref":"ev-0001"}]}"#;
const MISMATCH: &str = r#"{"recomputes":[{"signal_id":"sig-0001","claimed_rate":0.3,"recomputed_rate":0.45,"match":false,"evidence_ref":"ev-0001"}]}"#;

#[test]
fn corroborated_when_recompute_matches() {
    let out = run_with(Some(MATCH), &read_fixture("validation.in.json")).unwrap();
    assert_valid("validation.out", &out);
    assert!(out.contains("\"verdict\":\"corroborated\""), "{out}");
    assert!(out.contains("\"check\":\"denominator\",\"status\":\"pass\""), "{out}");
    assert!(out.contains("\"evidence_refs\":[\"ev-0001\"]"), "{out}");
}

#[test]
fn refuted_when_recompute_mismatches() {
    let out = run_with(Some(MISMATCH), &read_fixture("validation.in.json")).unwrap();
    assert_valid("validation.out", &out);
    assert!(out.contains("\"verdict\":\"refuted\""), "{out}");
    assert!(out.contains("\"check\":\"denominator\",\"status\":\"fail\""), "{out}");
}

#[test]
fn inconclusive_without_recompute_and_unevaluated_checks_are_not_applicable() {
    let out = run_with(None, &read_fixture("validation.in.json")).unwrap();
    assert_valid("validation.out", &out);
    assert!(out.contains("\"verdict\":\"inconclusive\""), "{out}");
    assert!(out.contains("\"check\":\"temporality\",\"status\":\"not_applicable\""), "{out}");
}

#[test]
fn scout_and_verifier_must_differ() {
    let bad = read_fixture("validation.in.json").replace("actor-verifier", "actor-scout");
    assert!(matches!(run_with(Some(MATCH), &bad), Err(StepError::Invalid(_))));
}

#[test]
fn rejects_unknown_check_and_bad_contract_version() {
    let bad = read_fixture("validation.in.json").replace("temporality", "vibes");
    assert!(matches!(run_with(Some(MATCH), &bad), Err(StepError::Invalid(_))));
    let bad = read_fixture("validation.in.json").replace("engine-steps/0", "engine-steps/9");
    assert!(matches!(run_with(Some(MATCH), &bad), Err(StepError::Invalid(_))));
}
