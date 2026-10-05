//! DET1: `candidate_exploratory` signals reach the loop labelled, capped, never as corroborated.
mod common;
use common::*;
use reasoning::finding::{Finding, Source};
use serde_json::json;

fn expl(reason: &str, with_holdout: bool) -> serde_json::Value {
    let mut s = signal("M1", json!({"reason_category": "Tecnico", "channel": "Web"}), stage(60, 264, 0.21), stage(50, 250, 0.21));
    s["status"] = json!("candidate_exploratory");
    s["reason"] = json!(reason);
    s["exploratory_note"] = json!("exploratory: weaker statistical evidence; the regression proof is the quality gate");
    if !with_holdout {
        s.as_object_mut().unwrap().remove("holdout");
    }
    s
}

#[test]
fn exploratory_findings_are_labelled_capped_and_never_corroborated() {
    let report = report_of(vec![
        signal("M1", json!({"reason_category": "Queja", "channel": "Phone"}), stage(300, 600, 0.2), stage(200, 400, 0.2)),
        expl("exploratory_discovery_only", false),
        expl("exploratory_holdout_weak", true),
        expl("exploratory_holdout_weak", true),
        expl("exploratory_holdout_weak", true),
    ]);
    // strict reading: no exploratory finding
    let (f0, s0) = Finding::from_report(&report, Source::Synthetic).unwrap();
    assert_eq!(f0.len(), 1);
    assert!(s0.iter().all(|s| s.status == "candidate_exploratory"));
    // cap 2: 2 exploratory taken, the third is capped, the one without holdout is skipped by name
    let (f, s) = Finding::from_report_with(&report, Source::Synthetic, 2).unwrap();
    assert_eq!(f.len(), 3);
    assert_eq!(f.iter().filter(|x| x.is_exploratory()).count(), 2);
    assert!(s.iter().any(|x| x.reason == "exploratory_cap"));
    assert!(s.iter().any(|x| x.reason == "exploratory_without_holdout"));
    let e = f.iter().find(|x| x.is_exploratory()).unwrap();
    let sig = e.to_signal_json();
    assert_eq!(sig["status"], "candidate_exploratory");
    assert!(sig["exploratory_note"].as_str().unwrap().starts_with("exploratory:"));
    let inputs = e.inputs();
    assert_eq!(inputs["status"], "candidate_exploratory");
    assert_eq!(inputs["replication"], "exploratory_label_not_gate");
    assert_ne!(f[0].inputs()["replication"], "exploratory_label_not_gate");
    assert_eq!(f[0].to_signal_json()["status"], "corroborated");
}
