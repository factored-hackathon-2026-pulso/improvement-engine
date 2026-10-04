//! Viability ledger: every proposal ends in a recorded verdict with a named reason, persisted through the engine store.
use engine::FileStore;
use engine::ledger::{GateResult, Ledger, LedgerEntry, Verdict, bk0_check};
use serde_json::json;

fn tmp(name: &str) -> FileStore {
    let d = std::env::temp_dir().join(format!("ledger-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    FileStore::open(d).unwrap()
}

fn gates(safety: &str, improvement: &str) -> Vec<GateResult> {
    vec![GateResult { gate: "safety".into(), status: safety.into() }, GateResult { gate: "improvement".into(), status: improvement.into() }]
}

fn viable() -> LedgerEntry {
    let mut e = LedgerEntry::new(0, "run-1", "prop-0", "sig-0001", Verdict::Viable, "structural_gate_passed");
    e.gates = gates("pass", "pass");
    e.gate_verdict = Some("pass".into());
    e.evidence_refs = vec!["ev-0001".into()];
    e.kind = Some("prompt".into());
    e.op = Some("replace".into());
    e
}

#[test]
fn bk0_supports_exactly_the_matrix_families() {
    assert_eq!(bk0_check("prompt", "replace"), Ok(()));
    assert_eq!(bk0_check("eval_suite", "add"), Ok(()));
    assert_eq!(bk0_check("prompt", "add"), Err("kind_not_supported".into()));
    assert_eq!(bk0_check("prompt", "disable"), Err("kind_not_supported".into()));
    for k in ["flow", "agent", "decision_model", "policy", "template", "tool", "language_detection", "injection_ruleset", "model_profile", "knowledge_snapshot", "no_such_kind"] {
        assert_eq!(bk0_check(k, "replace"), Err("kind_not_supported".into()), "{k}");
    }
    assert_eq!(bk0_check("release_settings", "replace"), Err("release_settings_not_allowed".into()));
}

#[test]
fn a_viable_verdict_needs_a_passed_gate_without_override_and_evidence() {
    assert_eq!(viable().validate(), Ok(()));
    let mut e = viable();
    e.gate_verdict = Some("fail".into());
    assert!(e.validate().unwrap_err().contains("viable"), "gate fail");
    let mut e = viable();
    e.gates = gates("pass", "fail");
    assert!(e.validate().is_err(), "a failing gate row");
    let mut e = viable();
    e.gate_verdict = None;
    assert!(e.validate().is_err(), "no gate result at all is never a pass");
    let mut e = viable();
    e.overrides = vec![json!({"label": "human_override", "simulated": true, "by": "human"})];
    assert!(e.validate().unwrap_err().contains("override"), "an override can never make a proposal viable");
    let mut e = viable();
    e.evidence_refs.clear();
    assert!(e.validate().is_err(), "no evidence");
    let mut e = viable();
    e.reason = "gate_failed".into();
    assert!(e.validate().is_err(), "reason must belong to the verdict");
}

#[test]
fn non_viable_and_not_evaluable_need_a_named_reason_of_their_own_class() {
    let ok = |v, r: &str| LedgerEntry::new(1, "run-1", "prop-1", "sig-2", v, r).validate();
    assert_eq!(ok(Verdict::NotViable, "gate_failed"), Ok(()));
    assert_eq!(ok(Verdict::NotEvaluable, "kind_not_supported"), Ok(()));
    assert_eq!(ok(Verdict::NotEvaluable, "evidence_insufficient"), Ok(()));
    assert!(ok(Verdict::NotViable, "kind_not_supported").is_err(), "an unevaluated proposal is not 'not viable'");
    assert!(ok(Verdict::NotEvaluable, "gate_failed").is_err());
    assert!(ok(Verdict::NotEvaluable, "").is_err());
    assert!(ok(Verdict::NotEvaluable, "because").is_err(), "reasons are a closed vocabulary");
}

#[test]
fn an_override_on_the_trail_must_be_labelled_simulated() {
    let mut e = LedgerEntry::new(0, "run-1", "prop-0", "sig-1", Verdict::NotViable, "gate_failed");
    e.gate_verdict = Some("fail".into());
    e.overrides = vec![json!({"label": "human_override", "simulated": true, "by": "human", "reason": "r"})];
    assert_eq!(e.validate(), Ok(()));
    e.overrides = vec![json!({"label": "human_override", "simulated": false})];
    assert!(e.validate().unwrap_err().contains("simulated"));
    e.overrides = vec![json!({"simulated": true})];
    assert!(e.validate().unwrap_err().contains("label"));
}

#[test]
fn entries_roundtrip_through_json() {
    let mut e = viable();
    e.hypotheses = vec![json!({"id": "h_1", "claimed_rate": 0.3})];
    e.models = vec![json!({"role": "scout", "label": "scripted", "real": false})];
    e.detail = "ok".into();
    let back = LedgerEntry::from_json(&e.to_json()).unwrap();
    assert_eq!(back, e);
    assert_eq!(e.to_json()["verdict"], "viable");
}

#[test]
fn the_ledger_persists_once_replays_identically_and_refuses_a_different_entry() {
    let s = tmp("persist");
    let l = Ledger::new(&s);
    let (a, mut b) = (viable(), LedgerEntry::new(1, "run-1", "prop-1", "sig-2", Verdict::NotEvaluable, "kind_not_supported"));
    b.kind = Some("flow".into());
    assert_eq!(l.record(&a), Ok(true));
    assert_eq!(l.record(&b), Ok(true));
    assert_eq!(l.record(&a), Ok(false), "an identical replay is a no-op");
    let mut changed = a.clone();
    changed.signal_id = "sig-other".into();
    assert!(l.record(&changed).unwrap_err().contains("immutable"));
    assert_eq!(l.entries().unwrap(), vec![a, b]);
    let mut invalid = viable();
    invalid.ordinal = 2;
    invalid.gate_verdict = Some("fail".into());
    assert!(l.record(&invalid).is_err(), "an invalid entry is never stored");
    assert_eq!(l.entries().unwrap().len(), 2);
}

#[test]
fn viable_needs_at_least_one_gate_row_and_a_bk0_supported_kind_and_op() {
    let mut e = viable();
    e.gates.clear();
    assert!(e.validate().is_err(), "a pass verdict with no gate rows proves nothing");
    for (k, o) in [(Some("flow"), Some("replace")), (Some("release_settings"), Some("replace")), (Some("prompt"), Some("add")), (None, Some("replace")), (Some("prompt"), None)] {
        let mut e = viable();
        e.kind = k.map(Into::into);
        e.op = o.map(Into::into);
        assert!(e.validate().unwrap_err().contains("BK0"), "{k:?}/{o:?} is not a supported change family");
    }
    let mut e = viable();
    e.kind = Some("eval_suite".into());
    e.op = Some("add".into());
    assert_eq!(e.validate(), Ok(()));
}

#[test]
fn a_not_viable_entry_with_an_unlabelled_or_unsimulated_override_is_refused_by_record() {
    let s = tmp("ovr");
    let mut e = LedgerEntry::new(0, "run-1", "prop-0", "sig-1", Verdict::NotViable, "gate_failed");
    e.overrides = vec![json!({"label": "human_override", "simulated": "true"})];
    assert!(Ledger::new(&s).record(&e).is_err(), "simulated must be the boolean true");
}
