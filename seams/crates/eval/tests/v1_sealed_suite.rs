use eval::suite::{EvalPackage, Suite, SuiteCase, SuiteError};
use serde_json::json;

fn suite() -> Suite {
    Suite {
        suite_id: "disputas-suite".into(),
        version: "1.0.0".into(),
        author_actor: "codex-suite".into(),
        cases: vec![
            SuiteCase { case_ref: "case-1".into(), scenario_manifest_ref: "art-golden".into(), oracle_ref: None, seed: json!(7) },
            SuiteCase { case_ref: "case-2".into(), scenario_manifest_ref: "art-golden".into(), oracle_ref: None, seed: json!(8) },
        ],
    }
}

fn params() -> eval::suite::RunParams {
    eval::suite::RunParams {
        binding_ref: "bind-arm".into(),
        budget_ref: "bud-1".into(),
        baseline_target: json!({"kind":"published_release","release_id":"rel-demo"}),
        candidate_target: json!({"kind":"frozen_candidate","candidate_hash":"c0ffee"}),
        repetitions: 2,
    }
}

#[test]
fn sealed_suite_test_fails_when_the_suite_changes_after_arms_started() {
    let s = suite();
    let mut p = EvalPackage::seal(s.clone());
    assert!(p.verify(&s).is_ok());
    let reqs = p.start_arms(&params()).unwrap();
    assert_eq!(reqs.len(), 2 * 2 * 2);
    let mut changed = s.clone();
    changed.cases[0].seed = json!(99);
    assert_eq!(p.verify(&changed), Err(SuiteError::ChangedAfterSeal));
    let mut extra = s.clone();
    extra.cases.pop();
    assert_eq!(p.verify(&extra), Err(SuiteError::ChangedAfterSeal));
    assert!(p.verify(&s).is_ok());
}

#[test]
fn arms_cannot_start_twice_and_requests_follow_single_flight_rules() {
    let mut p = EvalPackage::seal(suite());
    let reqs = p.start_arms(&params()).unwrap();
    assert_eq!(p.start_arms(&params()), Err(SuiteError::AlreadyStarted));
    let mut digests = std::collections::BTreeSet::new();
    for r in &reqs {
        r.validate().unwrap();
        assert!(digests.insert(r.single_flight_digest(None).unwrap()), "digest collision");
    }
    // deadline is a per-attempt bound: not part of the identity
    let mut r2 = reqs[0].clone();
    r2.deadline = Some("2030-01-01T00:00:00Z".into());
    assert_eq!(reqs[0].single_flight_digest(None).unwrap(), r2.single_flight_digest(None).unwrap());
    // deterministic: same sealed suite and params => same keys
    let again = EvalPackage::seal(suite()).start_arms(&params()).unwrap();
    assert_eq!(reqs, again);
    assert!(reqs.iter().any(|r| r.arm == "baseline") && reqs.iter().any(|r| r.arm == "candidate"));
}

#[test]
fn empty_or_duplicate_case_suites_are_not_sealable() {
    let mut s = suite();
    s.cases.clear();
    assert!(EvalPackage::try_seal(s).is_err());
    let mut d = suite();
    d.cases[1].case_ref = "case-1".into();
    assert!(EvalPackage::try_seal(d).is_err());
}
