//! `platform_aligned` profile: the engine's maturity measures lined up with the platform S21 `StageRule`
//! (support-platform `domain/ai/maturity.py`). One test per difference; the default profile is pinned unchanged.
use maturity::*;
use serde_json::json;

struct Fixed(Vec<(&'static str, TypeInputs)>);
impl InputSource for Fixed {
    fn source(&self) -> Source {
        Source::PlatformEvents
    }
    fn simulated(&self) -> bool {
        false
    }
    fn type_ids(&self) -> Vec<String> {
        self.0.iter().map(|(k, _)| k.to_string()).collect()
    }
    fn inputs(&self, t: &str) -> Option<TypeInputs> {
        self.0.iter().find(|(k, _)| *k == t).map(|(_, v)| v.clone())
    }
}

fn eval(t: &Thresholds, i: TypeInputs) -> Maturity {
    evaluate("undue_charge", t, &[&Fixed(vec![("undue_charge", i)])])
}

fn drafts(accepted: usize, edited_or_discarded: usize) -> Vec<Disposition> {
    let mut v = vec![Disposition::AsIs; accepted];
    v.extend(vec![Disposition::Discarded; edited_or_discarded]);
    v
}

#[test]
fn the_default_profile_is_unchanged() {
    let t = Thresholds::default();
    assert_eq!((t.resolved_min_cases, t.stage1_basis), (None, Stage1Basis::RepeatedQuestions));
    // any copilot question is stage 1, and 20 repeated-question cases are stage 2, whatever the resolved/asked counts say
    let m = eval(&t, TypeInputs { copilot_questions: Some(1), ..Default::default() });
    assert_eq!(m.stage, Stage::S1);
    let m = eval(&t, TypeInputs { copilot_questions: Some(50), repeat_q_cases: Some(20), copilot_cases: Some(3), resolved_cases: Some(0), ..Default::default() });
    assert_eq!(m.stage, Stage::S2);
    let j = thresholds_to_json(&t);
    assert!(j.get("profile").is_none() && j.get("stage0_min_resolved").is_none(), "default json keeps its shape: {j}");
}

#[test]
fn profile_platform_aligned_sets_the_s21_rule() {
    let t = Thresholds::platform_aligned();
    assert_eq!((t.resolved_min_cases, t.stage1_basis), (Some(10), Stage1Basis::CasesWithQuestions));
    assert_eq!((t.repeat_q_min_cases, t.tool_use_min, t.k_min, t.draft_accept_min, t.draft_window), (20, 0.7, 10, 0.8, 100));
    assert_eq!(thresholds_to_json(&t)["profile"], "platform_aligned");
}

#[test]
fn zero_to_one_needs_ten_resolved_cases_not_a_copilot_question() {
    let t = Thresholds::platform_aligned();
    // a copilot question alone no longer lifts the type
    assert_eq!(eval(&t, TypeInputs { copilot_questions: Some(80), resolved_cases: Some(9), ..Default::default() }).stage, Stage::S0);
    // ten resolved cases do, even without any copilot question
    assert_eq!(eval(&t, TypeInputs { resolved_cases: Some(10), ..Default::default() }).stage, Stage::S1);
    // no resolved-case count is a stated gap, never a guess
    let m = eval(&t, TypeInputs { copilot_questions: Some(80), ..Default::default() });
    assert_eq!(m.stage, Stage::S0);
    assert_eq!(m.blocked_by.as_deref(), Some("resolved_cases: no_resolved_cases"));
}

#[test]
fn one_to_two_counts_cases_with_questions_not_repeated_questions() {
    let t = Thresholds::platform_aligned();
    let base = TypeInputs { resolved_cases: Some(10), ..Default::default() };
    let repeated_only = TypeInputs { repeat_q_cases: Some(41), copilot_cases: Some(19), ..base.clone() };
    assert_eq!(eval(&t, repeated_only.clone()).stage, Stage::S1, "41 repeated-question cases do not count on the platform");
    assert_eq!(eval(&Thresholds::default(), TypeInputs { copilot_questions: Some(60), ..repeated_only }).stage, Stage::S2, "the default profile still counts them");
    let m = eval(&t, TypeInputs { copilot_cases: Some(20), ..base.clone() });
    assert_eq!(m.stage, Stage::S2);
    match m.repeat_q {
        Metric::Value { numerator, .. } => assert_eq!(numerator, 20, "the stage-1 metric is the cases with questions"),
        other => panic!("{other:?}"),
    }
    assert_eq!(eval(&t, base).blocked_by.as_deref(), Some("repeat_q: no_copilot_cases"));
}

#[test]
fn two_to_three_is_seventy_percent_with_ten_cases_in_both_profiles() {
    for t in [Thresholds::default(), Thresholds::platform_aligned()] {
        let mk = |used, applicable| TypeInputs { copilot_questions: Some(99), resolved_cases: Some(10), repeat_q_cases: Some(20), copilot_cases: Some(20), tool_used: Some(used), tool_applicable: Some(applicable), ..Default::default() };
        assert_eq!(eval(&t, mk(7, 10)).stage, Stage::S3);
        assert_eq!(eval(&t, mk(6, 10)).stage, Stage::S2);
        assert_eq!(eval(&t, mk(9, 9)).stage, Stage::S2, "fewer than 10 applicable cases never counts");
    }
}

#[test]
fn edited_drafts_count_as_discarded_and_three_to_agent_is_eighty_of_a_hundred() {
    let t = Thresholds::platform_aligned();
    let mk = |d| TypeInputs { resolved_cases: Some(10), copilot_cases: Some(20), tool_used: Some(8), tool_applicable: Some(10), drafts: Some(d), ..Default::default() };
    assert!(eval(&t, mk(drafts(80, 20))).agent_proposed);
    assert!(!eval(&t, mk(drafts(79, 21))).agent_proposed);
    // the adapter maps the platform draft outcome `edited` (a larger edit) to `discarded`
    let list: Vec<&str> = std::iter::repeat("as_is").take(79).chain(std::iter::repeat("edited").take(21)).collect();
    let v = json!({"case_types": [{"type_id": "undue_charge", "resolved_cases": 10, "copilot_cases": 20, "tool_used": 8, "tool_applicable": 10, "drafts": list}]});
    let src = JsonSource::from_json(Source::PlatformEvents, false, &v).unwrap();
    let m = evaluate("undue_charge", &t, &[&src]);
    assert_eq!(m.stage, Stage::S3);
    assert!(!m.agent_proposed, "79 accepted of 100 with 21 edited");
    let v2 = json!({"case_types": [{"type_id": "x", "drafts": ["edited"]}]});
    let d = JsonSource::from_json(Source::PlatformEvents, false, &v2).unwrap().inputs("x").unwrap().drafts.unwrap();
    assert_eq!(d, vec![Disposition::Discarded]);
}

#[test]
fn stage_and_agent_status_map_to_the_platform_names() {
    let t = Thresholds::platform_aligned();
    let m = eval(&t, TypeInputs { resolved_cases: Some(10), copilot_cases: Some(20), tool_used: Some(8), tool_applicable: Some(10), drafts: Some(drafts(80, 20)), ..Default::default() });
    assert_eq!((m.platform_stage(), m.platform_agent_status()), (3, "ready"));
    let m = eval(&t, TypeInputs { has_agent: true, ..Default::default() });
    assert_eq!((m.platform_stage(), m.platform_agent_status()), (3, "active"));
    let m = eval(&t, TypeInputs { resolved_cases: Some(10), ..Default::default() });
    assert_eq!((m.platform_stage(), m.platform_agent_status()), (1, "none"));
    assert_eq!(eval(&t, TypeInputs::default()).platform_stage(), 0);
}

#[test]
fn the_profile_is_configurable_from_json_and_overlays_apply_after_it() {
    let b = Thresholds::default();
    let t = thresholds_from_json(&b, &json!({"profile": "platform_aligned", "repeat_q_min_cases": 25})).unwrap();
    assert_eq!((t.resolved_min_cases, t.stage1_basis, t.repeat_q_min_cases), (Some(10), Stage1Basis::CasesWithQuestions, 25));
    let back = thresholds_from_json(&t, &json!({"profile": "default"})).unwrap();
    assert_eq!(back, Thresholds::default());
    assert!(thresholds_from_json(&b, &json!({"profile": "other"})).is_err());
    let custom = thresholds_from_json(&b, &json!({"stage0_min_resolved": 15, "stage1_basis": "cases_with_questions"})).unwrap();
    assert_eq!((custom.resolved_min_cases, custom.stage1_basis), (Some(15), Stage1Basis::CasesWithQuestions));
    assert!(thresholds_from_json(&b, &json!({"stage1_basis": "nope"})).is_err());
    assert!(thresholds_from_json(&b, &json!({"stage0_min_resolved": 0})).is_err());
    // the json of an aligned profile round-trips
    assert_eq!(thresholds_from_json(&b, &thresholds_to_json(&t)).unwrap(), t);
}

#[test]
fn json_rows_carry_resolved_and_copilot_cases() {
    let v = json!({"case_types": [{"type_id": "undue_charge", "resolved_cases": 12, "copilot_cases": 30}]});
    let i = JsonSource::from_json(Source::PlatformEvents, false, &v).unwrap().inputs("undue_charge").unwrap();
    assert_eq!((i.resolved_cases, i.copilot_cases), (Some(12), Some(30)));
}
