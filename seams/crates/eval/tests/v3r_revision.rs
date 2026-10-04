use eval::gate::{GateResult, GateStatus, GateVerdict, Verdict};
use eval::revision::*;

fn gv(v: Verdict) -> GateVerdict {
    let st = match v {
        Verdict::Pass => GateStatus::Pass,
        Verdict::Fail => GateStatus::Fail,
        Verdict::NotEvaluable => GateStatus::NotEvaluable,
    };
    GateVerdict {
        verdict: v,
        gates: vec![
            GateResult { gate: "safety".into(), status: st.clone(), reason: Some("r".into()) },
            GateResult { gate: "improvement".into(), status: st, reason: None },
        ],
        judge_actor: "claude-standin".into(),
        label: "gate=claude-authored".into(),
        semantics: "claude-standin".into(),
        quality_claims: "forbidden".into(),
        suite_ref: "eval_suite:x@1".into(),
    }
}
fn spec() -> ChangeSpec {
    ChangeSpec { variant: "catalogue:rename-guard@1".into(), max_files: 8, max_lines: 200 }
}
const CEIL: u64 = 10_000;

#[test]
fn failure_leads_to_revision_then_stop_budget_unchanged() {
    let mut calls = 0;
    let out = run_bounded("run-1", spec(), CEIL, &ShrinkPolicy, |_a| {
        calls += 1;
        Ok((gv(Verdict::Fail), 1_000))
    })
    .unwrap();
    assert_eq!(calls, 3, "initial + revision 1 + revision 2, no third revision");
    assert_eq!(out.stop, Stop::RevisionsExhausted);
    let labels: Vec<_> = out.attempts.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, ["initial", "revision-1", "revision-2"]);
    assert_eq!(out.budget_ceiling, CEIL, "ceiling never grows");
    assert_eq!(out.spent, 3_000);
    assert!(out.policy_label.contains("rule-driven stand-in"));
}

#[test]
fn pass_stops_without_revision() {
    let mut calls = 0;
    let out = run_bounded("run-1", spec(), CEIL, &ShrinkPolicy, |_| {
        calls += 1;
        Ok((gv(Verdict::Pass), 10))
    })
    .unwrap();
    assert_eq!((calls, out.stop), (1, Stop::Passed));
}

#[test]
fn no_verdict_revises_then_pass_on_revision_two() {
    let mut n = 0;
    let out = run_bounded("run-1", spec(), CEIL, &ShrinkPolicy, |_| {
        n += 1;
        Ok((gv(if n == 1 { Verdict::NotEvaluable } else if n == 2 { Verdict::Fail } else { Verdict::Pass }), 5))
    })
    .unwrap();
    assert_eq!(out.stop, Stop::Passed);
    assert_eq!(out.attempts.len(), 3);
    assert_eq!(out.attempts[2].spec.max_files, 2);
    assert_eq!(out.attempts[2].spec.variant, "catalogue:rename-guard@1+r2");
}

#[test]
fn idempotency_keys_distinct_per_attempt_and_stable() {
    let run = || run_bounded("run-1", spec(), CEIL, &ShrinkPolicy, |_| Ok((gv(Verdict::Fail), 1))).unwrap();
    let (a, b) = (run(), run());
    let keys: std::collections::BTreeSet<_> = a.attempts.iter().map(|x| &x.idempotency_key).collect();
    assert_eq!(keys.len(), 3);
    assert_eq!(a, b);
}

#[test]
fn ceiling_stops_before_next_attempt_and_never_grows() {
    let out = run_bounded("run-1", spec(), 1_500, &ShrinkPolicy, |_| Ok((gv(Verdict::Fail), 1_000))).unwrap();
    assert_eq!(out.stop, Stop::BudgetExhausted);
    assert_eq!(out.attempts.len(), 2);
    assert_eq!(out.budget_ceiling, 1_500);
}

#[test]
fn evaluator_error_propagates() {
    assert!(run_bounded("r", spec(), CEIL, &ShrinkPolicy, |_| Err("infra".into())).is_err());
}

#[test]
fn judge_not_separated_stops_without_burning_attempts() {
    let mut calls = 0;
    let out = run_bounded("r", spec(), CEIL, &ShrinkPolicy, |_| {
        calls += 1;
        let mut v = gv(Verdict::NotEvaluable);
        for g in &mut v.gates {
            g.reason = Some("judge_not_separated".into());
        }
        Ok((v, 1))
    })
    .unwrap();
    assert_eq!(calls, 1, "a revision cannot fix author==judge");
    assert_eq!(out.stop, Stop::JudgeNotSeparated);
}

#[test]
fn shrink_never_widens_and_stops_when_it_cannot_narrow() {
    let zero = ChangeSpec { variant: "v".into(), max_files: 0, max_lines: 0 };
    if let Some(s) = ShrinkPolicy.revise(&zero, &[], 1) {
        assert!(s.max_files <= zero.max_files && s.max_lines <= zero.max_lines, "revision widened a zero limit");
    }
    let one = ChangeSpec { variant: "v".into(), max_files: 1, max_lines: 1 };
    assert!(ShrinkPolicy.revise(&one, &[], 1).is_none(), "identical limits are not a revision");
}
