//! R1G: Rust sensor over event packages (`sensor=rust-events`). Planted R1S-shaped scenarios, honest discards.
mod ev_common;
use ev_common::*;
use steps::events_sensor::{Params, Report, analyze};
use steps::sensor::json::{self, Json};

fn p() -> Params {
    // the generator compresses weeks into hours, so the day gate is lowered here (the gate itself is tested below)
    Params { min_history_days: 0, min_history_cases: 200, ..Params::default() }
}
fn admitted<'a>(r: &'a Report, family: &str) -> Vec<&'a str> {
    r.signals.iter().filter(|s| s.family == family).map(|s| s.cell.as_str()).collect()
}
fn reasons(r: &Report) -> Vec<&str> {
    r.discards.iter().map(|d| d.reason.as_str()).collect()
}

#[test]
fn escalation_rise_is_admitted_in_the_planted_cell_only() {
    let d = make(Scn::Escalation, 1, 3000);
    let r = analyze(&d.events, &d.cases, &p());
    assert_eq!(admitted(&r, "reassignment_rate"), vec!["pt/web_chat"], "{r:?}");
    assert!(r.signals.iter().all(|s| s.family == "reassignment_rate"), "no other family fires: {r:?}");
    let s = &r.signals[0];
    assert!(s.denominator >= 100 && s.numerator >= 20 && s.numerator <= s.denominator);
    assert!(s.metric_id.starts_with("reassignment_rate"), "{}", s.metric_id);
}

#[test]
fn recurrence_rise_is_admitted_in_the_planted_cell_only() {
    let d = make(Scn::Recurrence, 2, 3000);
    let r = analyze(&d.events, &d.cases, &p());
    assert_eq!(admitted(&r, "recurrence_rate"), vec!["pt/web_chat"], "{r:?}");
    assert!(r.signals.iter().all(|s| s.family == "recurrence_rate"), "{r:?}");
}

#[test]
fn null_scenario_admits_nothing_across_20_seeds_and_reports_the_measured_rate() {
    let mut runs_with_admission = 0;
    let mut screened_out = 0;
    for seed in 1..=20u64 {
        let d = make(Scn::Null, 100 + seed, 2500);
        let r = analyze(&d.events, &d.cases, &p());
        if !r.signals.is_empty() {
            runs_with_admission += 1;
        }
        screened_out += r.discards.iter().filter(|x| matches!(x.reason.as_str(), "not_replicated" | "multiple_comparison")).count();
        assert!(r.tests_run >= 8, "tests actually ran: {}", r.tests_run);
    }
    eprintln!("R1G null: admitted false-positive runs {runs_with_admission}/20 = {:.0}%; screened-out candidates {screened_out}", runs_with_admission as f64 * 5.0);
    assert!(runs_with_admission as f64 / 20.0 <= 0.05, "admitted FPR {runs_with_admission}/20");
}

#[test]
fn volume_drift_is_reported_as_drift_not_as_an_improvement_opportunity() {
    let d = make(Scn::VolumeDrift, 3, 3000);
    let r = analyze(&d.events, &d.cases, &p());
    assert!(r.signals.is_empty(), "drift must not be admitted: {r:?}");
    assert!(!r.drift.is_empty(), "drift reported: {r:?}");
    assert!(reasons(&r).contains(&"drift_only"), "{:?}", r.discards);
}

#[test]
fn cold_start_shorter_than_the_gate_is_insufficient_history_and_admits_nothing() {
    let d = make(Scn::Escalation, 1, 120);
    let r = analyze(&d.events, &d.cases, &p());
    assert!(r.signals.is_empty());
    assert!(!r.discards.is_empty() && r.discards.iter().all(|x| x.reason == "insufficient_history"), "{:?}", r.discards);
    // day gate: plenty of cases but a span of hours is below a 14 day gate
    let d = make(Scn::Escalation, 1, 3000);
    let strict = Params { min_history_days: 14, ..p() };
    let r = analyze(&d.events, &d.cases, &strict);
    assert!(r.signals.is_empty() && reasons(&r).contains(&"insufficient_history"), "{r:?}");
}

#[test]
fn a_cell_below_k_is_never_reported_and_is_named_k_anonymity() {
    let mut d = make(Scn::Null, 5, 2500);
    // 8 extra cases in a tiny cell, all reassigned (extreme rate, below k = 10)
    let mut seq = d.events.lines().count() as i64;
    let mut extra = String::new();
    for i in 0..8 {
        let cid = format!("CAS-tiny{i}");
        d.cases.push_str(&format!("{{\"case_id\":\"{cid}\",\"channel\":\"voice\",\"language\":\"xx\",\"previous_case_id\":null}}\n"));
        for (ty, role) in [("case.opened", "customer"), ("case.assigned", "system"), ("case.assigned", "supervisor")] {
            seq += 1;
            extra.push_str(&format!("{{\"sequence\":{seq},\"event_id\":\"EVT-tiny{seq}\",\"event_type\":\"{ty}\",\"case_id\":\"{cid}\",\"actor_role\":\"{role}\",\"actor_id\":null,\"event_time\":\"2026-09-09T00:00:00Z\"}}\n"));
        }
    }
    d.events.push_str(&extra);
    let r = analyze(&d.events, &d.cases, &p());
    assert!(r.signals.iter().all(|s| s.cell != "xx/voice"), "{r:?}");
    assert!(r.discards.iter().any(|x| x.cell == "xx/voice" && x.reason == "k_anonymity"), "{:?}", r.discards);
}

#[test]
fn hostile_and_malformed_rows_are_quarantined_and_do_not_stop_the_analysis() {
    let d = make(Scn::Escalation, 1, 3000);
    let mut ev = d.events.clone();
    let mut cs = d.cases.clone();
    let deep = "[".repeat(200);
    let hostile = [
        "not json at all".to_string(),
        "{\"sequence\":\"x\",\"event_type\":1}".to_string(),
        "{\"sequence\":-5,\"event_id\":\"E\",\"event_type\":\"case.opened\",\"event_time\":\"2026-09-02T00:00:00Z\"}".to_string(),
        "{\"sequence\":99999999,\"event_id\":\"E1\",\"event_type\":\"case.opened\",\"event_time\":\"yesterday\"}".to_string(),
        "{\"sequence\":1,\"event_id\":\"dup\",\"event_type\":\"case.opened\",\"event_time\":\"2026-09-02T00:00:00Z\"}".to_string(),
        deep,
        "{\"sequence\":7,\"event_id\":\"\\u0000\",\"event_type\":\"<script>alert(1)</script>\",\"event_time\":\"2026-09-02T00:00:00Z\"}".to_string(),
        format!("{{\"sequence\":123456789,\"event_id\":\"{}\",\"event_type\":\"case.opened\",\"event_time\":\"2026-09-02T00:00:00Z\"}}", "A".repeat(100_000)),
    ];
    for bad in hostile {
        ev.push_str(&bad);
        ev.push('\n');
    }
    cs.push_str("{\"case_id\":\"CAS-evil\",\"channel\":\"<img src=x>\",\"language\":\"es; DROP TABLE\",\"previous_case_id\":null}\ngarbage\n{\"case_id\":5}\n");
    let r = analyze(&ev, &cs, &p());
    assert!(r.quarantined >= 8, "quarantined {}", r.quarantined);
    assert_eq!(admitted(&r, "reassignment_rate"), vec!["pt/web_chat"], "{r:?}");
    let out = r.steps_output("run-hostile", "treated") + &r.to_json();
    assert!(!out.contains("<script>") && !out.contains("DROP TABLE") && !out.contains("<img"), "hostile text never echoed");
}

#[test]
fn output_never_carries_per_person_or_per_case_identifiers() {
    let d = make(Scn::Escalation, 1, 3000);
    let r = analyze(&d.events, &d.cases, &p());
    let out = r.steps_output("run-priv", "treated") + &r.to_json();
    assert!(r.signals.len() == 1);
    for id in d.ids.iter().step_by(7) {
        assert!(!out.contains(id.as_str()), "{id} leaked");
    }
    assert!(!out.contains("CAS-") && !out.contains("CUS-") && !out.contains("STF-") && !out.contains("EVT-"));
}

#[test]
fn output_is_deterministic_and_independent_of_row_order() {
    let d = make(Scn::Escalation, 1, 3000);
    let a = analyze(&d.events, &d.cases, &p());
    let b = analyze(&d.events, &d.cases, &p());
    assert_eq!(a.steps_output("run-det", "treated"), b.steps_output("run-det", "treated"));
    assert_eq!(a.to_json(), b.to_json());
    let mut lines: Vec<&str> = d.events.lines().collect();
    lines.reverse();
    let c = analyze(&(lines.join("\n") + "\n"), &d.cases, &p());
    assert_eq!(a.steps_output("run-det", "treated"), c.steps_output("run-det", "treated"));
}

#[test]
fn steps_output_is_the_frozen_engine_steps_shape() {
    let d = make(Scn::Escalation, 1, 3000);
    let r = analyze(&d.events, &d.cases, &p());
    let v = json::parse(&r.steps_output("run-shape", "treated")).unwrap();
    assert_eq!(v.get("contract_version").and_then(Json::as_str), Some("engine-steps/0"));
    assert_eq!(v.get("step").and_then(Json::as_str), Some("sensors"));
    let id_ok = |s: &str| (3..=64).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'_' || c == b'-') && !s.starts_with(['.', '_', '-']);
    let sigs = v.get("signals").and_then(Json::as_arr).unwrap();
    assert!(!sigs.is_empty());
    for s in sigs {
        for k in ["signal_id", "metric_id", "evidence_ref"] {
            assert!(id_ok(s.get(k).and_then(Json::as_str).unwrap()), "{k}");
        }
        assert_eq!(s.get("holdout_checked"), Some(&Json::Bool(true)));
        assert!(s.get("denominator").and_then(Json::as_i64).unwrap() >= 1);
        assert!(s.get("population").and_then(Json::as_str).is_some_and(|x| !x.is_empty()));
    }
    let frozen = ["below_k", "failed_holdout", "low_coverage", "duplicate", "other"];
    let dd = make(Scn::VolumeDrift, 3, 3000);
    let v2 = json::parse(&analyze(&dd.events, &dd.cases, &p()).steps_output("run-shape", "treated")).unwrap();
    let dsc = v2.get("discards").and_then(Json::as_arr).unwrap();
    assert!(!dsc.is_empty());
    for x in dsc {
        assert!(id_ok(x.get("metric_id").and_then(Json::as_str).unwrap()));
        assert!(frozen.contains(&x.get("reason").and_then(Json::as_str).unwrap()));
    }
}

#[test]
fn report_states_what_the_sensor_did_not_do() {
    let d = make(Scn::Null, 9, 2500);
    let r = analyze(&d.events, &d.cases, &p());
    let j = r.to_json();
    assert!(j.contains("\"sensor\":\"rust-events\"") && j.contains("sensor-events/1"), "{j}");
    assert!(j.contains("not_done") && j.contains("payload"), "{j}");
    assert!(j.contains("bonferroni"), "method documented: {j}");
}
