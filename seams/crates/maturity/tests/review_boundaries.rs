//! Adversarial review: exact threshold boundaries, window of 100, k floor, regression, determinism, simulated labelling.
use maturity::*;

struct Fixed(Source, bool, TypeInputs);
impl InputSource for Fixed {
    fn source(&self) -> Source {
        self.0
    }
    fn simulated(&self) -> bool {
        self.1
    }
    fn type_ids(&self) -> Vec<String> {
        vec!["t".into()]
    }
    fn inputs(&self, _: &str) -> Option<TypeInputs> {
        Some(self.2.clone())
    }
}
fn mk(repeat: u32, used: u32, applicable: u32, accepted: usize, total: usize) -> TypeInputs {
    let mut d = vec![Disposition::Discarded; total - accepted];
    d.extend(vec![Disposition::AsIs; accepted]);
    TypeInputs { copilot_questions: Some(500), repeat_q_cases: Some(repeat), tool_applicable: Some(applicable), tool_used: Some(used), drafts: Some(d), ..Default::default() }
}
fn eval(i: TypeInputs) -> Maturity {
    evaluate("t", &Thresholds::default(), &[&Fixed(Source::SimDraftStream, true, i)])
}

#[test]
fn exact_boundaries_pass_one_below_fails() {
    assert_eq!(eval(mk(20, 7, 10, 80, 100)).stage, Stage::S3);
    assert!(eval(mk(20, 7, 10, 80, 100)).agent_proposed);
    assert_eq!(eval(mk(19, 7, 10, 80, 100)).stage, Stage::S1);
    assert_eq!(eval(mk(20, 6, 10, 80, 100)).stage, Stage::S2);
    assert!(!eval(mk(20, 7, 10, 79, 100)).agent_proposed);
    assert_eq!(eval(mk(20, 70, 100, 80, 100)).stage, Stage::S3); // 0.7 exactly, other k
    assert_eq!(eval(mk(20, 7, 9, 80, 100)).stage, Stage::S2); // k floor: 9 < 10 is suppressed
}

#[test]
fn window_is_the_last_100_not_the_whole_history() {
    // 101 drafts: oldest (index 0) discarded, last 100 are 80 accepted -> passes; and with 79 in the last 100 it fails.
    let mut d = vec![Disposition::Discarded; 1];
    d.extend(vec![Disposition::Discarded; 20]);
    d.extend(vec![Disposition::Minor; 80]);
    let mut i = mk(20, 7, 10, 80, 100);
    i.drafts = Some(d);
    assert!(eval(i.clone()).agent_proposed);
    i.drafts = Some(vec![Disposition::Minor; 99]);
    assert!(matches!(eval(i).draft_accept_100, Metric::NotComputable { .. }));
}

#[test]
fn never_skips_and_regresses_when_metrics_fall() {
    // strong draft signal but weak tool use must NOT reach stage 3 or agent
    assert_eq!(eval(mk(20, 1, 10, 100, 100)).stage, Stage::S2);
    assert_eq!(eval(mk(5, 10, 10, 100, 100)).stage, Stage::S1);
    assert!(!eval(mk(5, 10, 10, 100, 100)).agent_proposed);
    let hi = eval(mk(30, 9, 10, 90, 100));
    let lo = eval(mk(30, 9, 10, 10, 100));
    assert_eq!((hi.stage, lo.stage), (Stage::S3, Stage::S3));
    assert!(hi.agent_proposed && !lo.agent_proposed);
    let down = eval(mk(10, 9, 10, 90, 100));
    assert_eq!(down.stage, Stage::S1);
    assert_eq!(eval(mk(30, 9, 10, 90, 100)), eval(mk(30, 9, 10, 90, 100)));
}

#[test]
fn a_simulated_agent_is_flagged_simulated_even_without_counts() {
    let i = TypeInputs { has_agent: true, ..Default::default() };
    let m = eval(i);
    assert_eq!(m.stage, Stage::Agent);
    assert!(m.simulated, "has_agent came from the simulated source");
}

#[test]
fn thresholds_validation_edges() {
    let b = Thresholds::default();
    for bad in [r#"{"tool_use_min":0}"#, r#"{"tool_use_min":-0.1}"#, r#"{"tool_use_min":1.0001}"#, r#"{"draft_accept_min":2}"#, r#"{"repeat_q_min_cases":0}"#, r#"{"repeat_q_min_cases":-1}"#, r#"{"repeat_q_min_cases":99999999999}"#, r#"{"k_min":9}"#, r#"{"draft_window":0}"#, r#"{"tool_use_min":"0.7"}"#, r#"{"x":1}"#, r#"[]"#] {
        assert!(thresholds_from_json(&b, &serde_json::from_str(bad).unwrap()).is_err(), "{bad}");
    }
    assert!(thresholds_from_json(&b, &serde_json::from_str(r#"{"tool_use_min":1,"k_min":10}"#).unwrap()).is_ok());
}
