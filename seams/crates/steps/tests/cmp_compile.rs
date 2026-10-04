use std::fs;
use std::path::PathBuf;
use steps::compile::run;

fn cases() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cmp/cases")
}

fn input(name: &str) -> String {
    fs::read_to_string(cases().join(format!("{name}.in.json"))).unwrap()
}

#[test]
fn two_operation_spec_compiles() {
    let out = run(&input("two_ops_compiled")).expect("compiles");
    assert!(out.contains("\"status\":\"compiled\""));
    assert!(out.contains("\"compiler_label\":\"claude-standin\""));
    assert!(out.find("\"op\":\"replace\"").unwrap() < out.find("\"op\":\"add\"").unwrap());
}

#[test]
fn four_denied_reasons_are_named() {
    for (case, reason) in [
        ("denied_kind_disable", "kind_not_supported"),
        ("denied_kind_add_prompt", "kind_not_supported"),
        ("denied_precondition", "missing_precondition"),
        ("denied_outside_target", "outside_bridge"),
        ("denied_outside_route", "outside_bridge"),
        ("denied_outside_bridge_ref", "outside_bridge"),
        ("denied_outside_bundle", "outside_bridge"),
        ("denied_mutable_same_ref", "mutable_reference"),
        ("denied_mutable_no_new_ref", "mutable_reference"),
        ("denied_mutable_double_publish", "mutable_reference"),
    ] {
        let out = run(&input(case)).unwrap();
        assert!(out.contains("\"status\":\"denied\""), "{case}");
        assert!(out.contains(&format!("\"denied_reason\":\"{reason}\"")), "{case}: {out}");
        assert!(!out.contains("draft_plan"), "{case}");
    }
}

#[test]
fn schema_invalid_input_is_an_error_not_a_denial() {
    assert!(run(&input("frz0_invalid_unsupported_kind")).is_err());
    assert!(run(&input("invalid_extra_field")).is_err());
    assert!(run("not json").is_err());
}

#[test]
fn dry_run_digest_hook_wins_and_is_validated() {
    use steps::compile::{World, run_with};
    let w = World::seeded_base();
    let out = run_with(&input("replace_only"), &w, Some(&|_ops: &[String]| format!("sha256:{}", "b".repeat(64)))).unwrap();
    assert!(out.contains(&format!("\"digest\":\"sha256:{}\"", "b".repeat(64))));
    assert!(run_with(&input("replace_only"), &w, Some(&|_: &[String]| "nope".to_string())).is_err());
}

#[test]
fn digest_is_content_bound() {
    let a = run(&input("two_ops_compiled")).unwrap();
    let b = run(&input("replace_only")).unwrap();
    let d = |s: &str| s[s.find("\"digest\":").unwrap()..].to_string();
    assert_ne!(d(&a), d(&b));
    assert_eq!(a, run(&input("two_ops_compiled")).unwrap());
}
