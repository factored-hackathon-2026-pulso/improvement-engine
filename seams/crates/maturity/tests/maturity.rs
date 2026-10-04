use maturity::*;
use serde_json::json;

struct Fixed(Source, bool, Vec<(&'static str, TypeInputs)>);
impl InputSource for Fixed {
    fn source(&self) -> Source {
        self.0
    }
    fn simulated(&self) -> bool {
        self.1
    }
    fn type_ids(&self) -> Vec<String> {
        self.2.iter().map(|(k, _)| k.to_string()).collect()
    }
    fn inputs(&self, t: &str) -> Option<TypeInputs> {
        self.2.iter().find(|(k, _)| *k == t).map(|(_, v)| v.clone())
    }
}
fn drafts(a: u32, m: u32, d: u32) -> Vec<Disposition> {
    let mut v = vec![Disposition::AsIs; a as usize];
    v.extend(vec![Disposition::Minor; m as usize]);
    v.extend(vec![Disposition::Discarded; d as usize]);
    v
}
fn full() -> TypeInputs {
    TypeInputs {
        copilot_questions: Some(80),
        repeat_q_cases: Some(41),
        tool_applicable: Some(10),
        tool_used: Some(8),
        drafts: Some(drafts(61, 23, 16)),
        cases_today: Some(21),
        ..Default::default()
    }
}

#[test]
fn defaults_match_the_screens() {
    let t = Thresholds::default();
    assert_eq!((t.repeat_q_min_cases, t.draft_window, t.k_min), (20, 100, 10));
    assert_eq!((t.tool_use_min, t.draft_accept_min), (0.7, 0.8));
}

#[test]
fn cobro_indebido_reaches_stage3_and_agent_proposed_from_a_simulated_stream() {
    let s = Fixed(Source::SimDraftStream, true, vec![("cobro", full())]);
    let m = evaluate("cobro", &Thresholds::default(), &[&s]);
    assert_eq!(m.stage, Stage::S3);
    assert!(m.agent_proposed && m.simulated);
    match m.draft_accept_100 {
        Metric::Value { numerator, denominator, source, simulated, .. } => {
            assert_eq!((numerator, denominator, source, simulated), (84, 100, Source::SimDraftStream, true))
        }
        _ => panic!(),
    }
}

#[test]
fn tool_use_below_threshold_stays_stage2() {
    let mut i = full();
    i.tool_used = Some(6);
    let s = Fixed(Source::SimDraftStream, true, vec![("t", i)]);
    let m = evaluate("t", &Thresholds::default(), &[&s]);
    assert_eq!(m.stage, Stage::S2);
    assert!(!m.agent_proposed);
}

#[test]
fn few_cases_stays_stage1_and_none_is_stage0() {
    let mut i = full();
    i.repeat_q_cases = Some(19);
    let s = Fixed(Source::SimDraftStream, true, vec![("a", i), ("b", TypeInputs { copilot_questions: Some(0), ..Default::default() })]);
    assert_eq!(evaluate("a", &Thresholds::default(), &[&s]).stage, Stage::S1);
    assert_eq!(evaluate("b", &Thresholds::default(), &[&s]).stage, Stage::S0);
}

#[test]
fn draft_acceptance_needs_a_full_window_else_not_computable() {
    let mut i = full();
    i.drafts = Some(drafts(30, 10, 10));
    let s = Fixed(Source::SimDraftStream, true, vec![("t", i)]);
    let m = evaluate("t", &Thresholds::default(), &[&s]);
    assert_eq!(m.stage, Stage::S3);
    assert!(!m.agent_proposed);
    assert!(matches!(m.draft_accept_100, Metric::NotComputable { ref reason } if reason.contains("window")));
}

#[test]
fn e0_without_suggestion_rows_says_not_computable_and_never_fabricates() {
    let e0 = Fixed(
        Source::E0Treated,
        false,
        vec![("t", TypeInputs { copilot_questions: Some(200), repeat_q_cases: Some(154), tool_applicable: Some(154), tool_used: Some(120), drafts: None, ..Default::default() })],
    );
    let m = evaluate("t", &Thresholds::default(), &[&e0]);
    assert_eq!(m.stage, Stage::S3);
    assert!(!m.agent_proposed && !m.simulated);
    assert!(matches!(m.draft_accept_100, Metric::NotComputable { ref reason } if reason.contains("no_draft_rows")));
    assert_eq!(m.blocked_by.as_deref(), Some("draft_accept_100: no_draft_rows"));
    match m.repeat_q {
        Metric::Value { source, simulated, .. } => assert_eq!((source, simulated), (Source::E0Treated, false)),
        _ => panic!(),
    }
}

#[test]
fn per_metric_source_fallback_marks_the_stage_simulated_only_when_a_simulated_metric_is_used() {
    let e0 = Fixed(
        Source::E0Treated,
        false,
        vec![("t", TypeInputs { copilot_questions: Some(200), repeat_q_cases: Some(154), tool_applicable: Some(100), tool_used: Some(80), ..Default::default() })],
    );
    let sim = Fixed(Source::SimDraftStream, true, vec![("t", full())]);
    let m = evaluate("t", &Thresholds::default(), &[&e0, &sim]);
    assert!(m.agent_proposed && m.simulated);
    match m.tool_use_rate {
        Metric::Value { source, .. } => assert_eq!(source, Source::E0Treated),
        _ => panic!(),
    }
    let only_e0 = evaluate("t", &Thresholds::default(), &[&e0]);
    assert!(!only_e0.simulated);
}

#[test]
fn small_cells_are_suppressed_k_anonymity() {
    let mut i = full();
    i.tool_applicable = Some(5);
    i.tool_used = Some(5);
    let s = Fixed(Source::E0Treated, false, vec![("t", i)]);
    let m = evaluate("t", &Thresholds::default(), &[&s]);
    assert!(matches!(m.tool_use_rate, Metric::NotComputable { ref reason } if reason.contains("k_min")));
    assert_eq!(m.stage, Stage::S2);
}

#[test]
fn con_agente_reports_agent_resolved_of() {
    let i = TypeInputs { copilot_questions: Some(50), has_agent: true, agent_handled: Some(26), agent_resolved: Some(17), ..Default::default() };
    let s = Fixed(Source::SimDraftStream, true, vec![("t", i)]);
    let m = evaluate("t", &Thresholds::default(), &[&s]);
    assert_eq!(m.stage, Stage::Agent);
    assert!(matches!(m.agent_resolved, Metric::Value { numerator: 17, denominator: 26, .. }));
}

#[test]
fn unknown_type_is_stage0_not_computable() {
    let m = evaluate("zzz", &Thresholds::default(), &[]);
    assert_eq!(m.stage, Stage::S0);
    assert!(matches!(m.repeat_q, Metric::NotComputable { .. }));
}

#[test]
fn thresholds_roundtrip_and_validation() {
    let b = Thresholds::default();
    let t = thresholds_from_json(&b, &json!({"repeat_q_min_cases": 30, "tool_use_min": 0.6})).unwrap();
    assert_eq!((t.repeat_q_min_cases, t.tool_use_min, t.draft_accept_min), (30, 0.6, 0.8));
    assert_eq!(thresholds_to_json(&t)["repeat_q_min_cases"], 30);
    assert!(thresholds_from_json(&b, &json!({"tool_use_min": 1.5})).is_err());
    assert!(thresholds_from_json(&b, &json!({"draft_window": 0})).is_err());
    assert!(thresholds_from_json(&b, &json!({"repeat_q_min_cases": "x"})).is_err());
    assert!(thresholds_from_json(&b, &json!({"k_min": 1})).is_err());
}

#[test]
fn json_source_parses_aggregates_and_refuses_ids_or_bad_dispositions() {
    let v = json!({"case_types": [{"type_id": "cobro", "copilot_questions": 9, "repeat_q_cases": 30, "tool_applicable": 10, "tool_used": 7, "cases_today": 4,
        "drafts": ["as_is", "minor", "discarded"]}]});
    let s = JsonSource::from_json(Source::SimDraftStream, true, &v).unwrap();
    assert_eq!(s.type_ids(), vec!["cobro".to_string()]);
    assert_eq!(s.source(), Source::SimDraftStream);
    let i = s.inputs("cobro").unwrap();
    assert_eq!(i.drafts.unwrap().len(), 3);
    let bad = json!({"case_types": [{"type_id": "x", "case_id": "c1"}]});
    assert!(JsonSource::from_json(Source::E0Treated, false, &bad).is_err());
    let bad2 = json!({"case_types": [{"type_id": "x", "drafts": ["hola"]}]});
    assert!(JsonSource::from_json(Source::SimDraftStream, true, &bad2).is_err());
}

#[test]
fn to_json_carries_source_and_simulated_on_every_metric() {
    let s = Fixed(Source::SimDraftStream, true, vec![("cobro", full())]);
    let t = Thresholds::default();
    let j = evaluate("cobro", &t, &[&s]).to_json(&t);
    assert_eq!(j["stage"], 3);
    assert_eq!(j["metrics"]["draft_accept_100"]["source"], "sim_draft_stream");
    assert_eq!(j["metrics"]["draft_accept_100"]["simulated"], true);
    assert_eq!(j["metrics"]["agent_resolved"]["status"], "not_computable");
}

#[test]
fn display_only_counts_are_admitted_in_rows() {
    let v = json!({"case_types": [{"type_id": "x", "agent_handed": 7, "copilot_cases": 3, "cases_total": 9, "label": "X", "group": "G", "stage_since": {"1": "2026-08-04"}}]});
    let s = JsonSource::from_json(Source::SimDraftStream, true, &v).unwrap();
    assert_eq!(s.row("x").unwrap()["cases_total"], 9);
}
