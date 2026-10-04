use core_client::dto::ArmReport;
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
    GateInput { run_id: "run-v2-0001", judge_actor: judge, author_actors: vec!["codex-builder"], world_author: "codex-world", suite_author, base, candidate: cand }
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
