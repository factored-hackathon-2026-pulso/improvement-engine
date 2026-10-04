use core_client::dto::ArmReport;
const D0: &str = "abababababababababababababababababababababababababababababababab";
use eval::gate::{GateInput, GateStatus, Verdict, wire_gate};
use serde_json::json;

fn rep(case: &str, status: &str, oracle: bool) -> ArmReport {
    ArmReport::from_json(&json!({
        "execution_id": "arm-00000000000000000000000000000000", "status": status, "case_ref": case,
        "closed_early": false, "cost_known": true, "oracle_ref": if oracle { json!("oracle:o1@1") } else { json!(null) },
        "usage": {"cost_usd": "0.002", "jobs": 1}
    }))
    .unwrap()
}

fn input<'a>(base: &'a [ArmReport], cand: &'a [ArmReport], judge: &'a str, suite_author: &'a str) -> GateInput<'a> {
    GateInput { suite_digest: D0, run_id: "run-v2-0001", judge_actor: judge, author_actors: vec!["codex-builder"], world_author: "codex-world", suite_author, base, candidate: cand }
}

#[test]
fn author_equals_judge_test_fails() {
    let (b, c) = ([rep("case-1", "completed", true)], [rep("case-1", "completed", true)]);
    for (judge, suite_author) in [("claude-standin", "claude-standin"), ("codex-builder", "codex-suite")] {
        let v = wire_gate(&input(&b, &c, judge, suite_author)).unwrap();
        assert_eq!(v.verdict, Verdict::NotEvaluable, "{judge}");
        assert_eq!(v.gates.len(), 2);
        assert!(v.gates.iter().all(|g| g.status == GateStatus::NotEvaluable && g.reason.as_deref() == Some("judge_not_separated")));
    }
}

#[test]
fn separated_authors_give_a_verdict_from_both_gates_with_honest_labels() {
    let (b, c) = ([rep("case-1", "completed", true)], [rep("case-1", "completed", true)]);
    let v = wire_gate(&input(&b, &c, "claude-standin", "codex-suite")).unwrap();
    assert_eq!(v.gates.iter().map(|g| g.gate.as_str()).collect::<Vec<_>>(), ["safety", "improvement"]);
    assert_eq!(v.label, "gate=claude-authored");
    assert_eq!(v.semantics, "claude-standin");
    assert_eq!(v.quality_claims, "forbidden");
    assert_ne!(v.verdict, Verdict::NotEvaluable);
}

#[test]
fn completed_arms_without_an_oracle_are_not_evaluable_never_a_pass() {
    // live finding: Core arms report only status/closed_early/cost/usage (oracle_ref null)
    let (b, c) = ([rep("case-1", "completed", false)], [rep("case-1", "completed", false)]);
    let v = wire_gate(&input(&b, &c, "claude-standin", "codex-suite")).unwrap();
    assert_eq!(v.verdict, Verdict::NotEvaluable);
    assert!(v.gates.iter().all(|g| g.reason.as_deref() == Some("oracle_missing")));
}

#[test]
fn case_set_mismatch_is_not_evaluable() {
    let (b, c) = ([rep("case-1", "completed", true)], [rep("case-2", "completed", true)]);
    let v = wire_gate(&input(&b, &c, "claude-standin", "codex-suite")).unwrap();
    assert_eq!(v.verdict, Verdict::NotEvaluable);
}

#[test]
fn suite_ref_is_bound_to_the_sealed_suite_digest() {
    let (b, c) = ([rep("case-1", "completed", true)], [rep("case-1", "completed", true)]);
    let d1 = "ab".repeat(32);
    let d2 = "cd".repeat(32);
    let mut i = input(&b, &c, "claude-standin", "codex-suite");
    i.suite_digest = &d1;
    let v1 = wire_gate(&i).unwrap();
    i.suite_digest = &d2;
    let v2 = wire_gate(&i).unwrap();
    assert!(v1.suite_ref.contains(&d1) && v2.suite_ref.contains(&d2) && v1.suite_ref != v2.suite_ref);
    i.suite_digest = "not-a-digest";
    assert!(wire_gate(&i).is_err());
}
