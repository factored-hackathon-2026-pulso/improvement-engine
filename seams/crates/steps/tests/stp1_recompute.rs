mod stp1_common;
use std::sync::Mutex;
use steps::{StepError, recompute};
use stp1_common::*;

static ENV: Mutex<()> = Mutex::new(()); // env vars are process-wide

/// Run recompute with a lab file whose content is `lab_json` (stored as sample-1.json).
fn with_lab(lab_json: &str, input: &str) -> Result<String, StepError> {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("stp1-lab-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sample-1.json"), lab_json).unwrap();
    unsafe { std::env::set_var("STEPS_LAB_DIR", &dir) };
    recompute::run(input)
}

fn lab(numerator: i64, count: i64) -> String {
    format!(r#"{{"rows":[{{"signal_id":"sig-0001","evidence_ref":"ev-0001","numerator":{numerator},"count":{count}}}]}}"#)
}

#[test]
fn recompute_matches_scout_claim_and_validates_against_out_schema() {
    let out = with_lab(&read_fixture("lab-sample-1.json"), &read_fixture("recompute.in.json")).unwrap();
    assert_valid("recompute.out", &out);
    assert!(out.contains("\"match\":true"), "{out}");
    assert!(out.contains("\"recomputed_rate\":0.3"), "{out}");
    assert!(out.contains("\"evidence_ref\":\"ev-0001\""), "{out}");
}

#[test]
fn recompute_fails_on_a_tampered_numerator() {
    // scout claimed 0.3 (= 120/400); the lab row was tampered to 180/400
    let out = with_lab(&lab(180, 400), &read_fixture("recompute.in.json")).unwrap();
    assert_valid("recompute.out", &out);
    assert!(out.contains("\"match\":false"), "tampered numerator must not match: {out}");
    assert!(out.contains("\"recomputed_rate\":0.45"), "{out}");
}

#[test]
fn recompute_rounds_half_even_at_two_decimals() {
    // 5/8 = 0.625 -> 0.62 (half-even), 3/8 = 0.375 -> 0.38
    let c = read_fixture("recompute.in.json").replace("0.3", "0.62");
    let out = with_lab(&lab(5, 8), &c).unwrap();
    assert!(out.contains("\"recomputed_rate\":0.62") && out.contains("\"match\":true"), "{out}");
    let c = read_fixture("recompute.in.json").replace("0.3", "0.38");
    let out = with_lab(&lab(3, 8), &c).unwrap();
    assert!(out.contains("\"recomputed_rate\":0.38") && out.contains("\"match\":true"), "{out}");
}

#[test]
fn recompute_rejects_claim_out_of_range_and_unknown_signal() {
    let bad = read_fixture("recompute.in.json").replace("0.3", "1.5");
    assert!(matches!(with_lab(&lab(120, 400), &bad), Err(StepError::Invalid(_))));
    let unk = read_fixture("recompute.in.json").replace("sig-0001", "sig-9999");
    assert!(matches!(with_lab(&lab(120, 400), &unk), Err(StepError::Invalid(_))));
    assert!(matches!(with_lab(&lab(120, 0), &read_fixture("recompute.in.json")), Err(StepError::Invalid(_))));
}
