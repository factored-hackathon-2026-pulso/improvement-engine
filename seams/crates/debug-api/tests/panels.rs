//! The panel projection over a REAL committed payload of the offline thread (fixture dumped from thread10's `Run.payload`).
use debug_api::NewEvent;
use debug_api::panels::project;
use serde_json::{Value, json};

const AT: &str = "2026-10-04T12:00:00Z";

fn full() -> Value {
    serde_json::from_str(include_str!("fixtures/committed-demo0.json")).expect("fixture")
}

fn pick(evs: &[NewEvent], kind: &str) -> Value {
    evs.iter().find(|e| e.kind == kind).unwrap_or_else(|| panic!("no {kind} in {:?}", evs.iter().map(|e| &e.kind).collect::<Vec<_>>())).data.clone()
}

fn evidence_ids(inv: &Value) -> Vec<String> {
    inv["evidence"].as_array().unwrap().iter().map(|e| e["evidence_ref"]["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn investigation_has_the_scripted_claim_the_verifier_verdict_and_evidence_that_resolves() {
    let evs = project(&full(), None, AT);
    let inv = pick(&evs, "investigation_set");
    assert!(inv["hypothesis"].as_str().unwrap().contains("Scripted scout claims signal sig-0001 has rate 0.30"), "{inv}");
    assert_eq!(inv["verifier"], "corroborated");
    let ids = evidence_ids(&inv);
    let hyps = inv["hypotheses"].as_array().unwrap();
    assert_eq!(hyps[0]["hypothesis_id"], "main");
    for h in hyps {
        for r in h["evidence_refs"].as_array().unwrap() {
            assert!(ids.contains(&r.as_str().unwrap().to_string()), "evidence ref {r} resolves to a shown item ({ids:?})");
        }
    }
    let by = |rel: &str| inv["evidence"].as_array().unwrap().iter().filter(|e| e["relation"] == rel).count();
    assert!(by("supports") >= 1 && by("limits") >= 3, "supports {} limits {}", by("supports"), by("limits"));
    let text = inv.to_string();
    assert!(text.contains("independence is by id only") && text.contains("scripted"), "double labels stay visible");
    for e in inv["evidence"].as_array().unwrap() {
        let d = e["evidence_ref"]["digest"].as_str().unwrap();
        assert!(d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()), "real sha256 digest: {d}");
        assert_eq!(e["available_at"], AT);
    }
}

#[test]
fn the_change_hypothesis_is_not_supported_when_the_improvement_gate_failed_and_says_why_arms_are_identical() {
    let evs = project(&full(), None, AT);
    let inv = pick(&evs, "investigation_set");
    let h = &inv["hypotheses"].as_array().unwrap()[1];
    assert_eq!(h["hypothesis_id"], "change");
    assert_eq!(h["verdict"], "not_supported");
    let st = h["statement"].as_str().unwrap();
    assert!(st.contains("replace prompt:resumen_radicado@1 -> prompt:resumen_radicado@2") && st.contains("add eval_suite:disputas-tarea-suite@2"), "the statement names what the change produces, not only what it targets: {st}");
    let gate = inv["evidence"].as_array().unwrap().iter().find(|e| e["evidence_ref"]["id"] == "ev-gate-improvement").unwrap();
    assert_eq!(gate["relation"], "contradicts");
    let s = gate["summary"].as_str().unwrap();
    assert!(s.contains("no_structural_improvement") && s.contains("arms identical"), "{s}");
}

#[test]
fn a_refuted_claim_shows_counter_evidence_and_the_refuted_verdict() {
    let mut c = full();
    c["out"]["validation"]["verdict"] = json!("refuted");
    c["out"]["recompute"]["recomputes"][0]["match"] = json!(false);
    c["out"]["recompute"]["recomputes"][0]["recomputed_rate"] = json!(0.25);
    let inv = pick(&project(&c, None, AT), "investigation_set");
    assert_eq!(inv["verifier"], "refuted");
    assert_eq!(inv["hypotheses"][0]["verdict"], "refuted");
    let rec = inv["evidence"].as_array().unwrap().iter().find(|e| e["evidence_ref"]["id"] == "ev-recompute-sig-0001").unwrap();
    assert_eq!(rec["relation"], "contradicts");
    assert!(rec["summary"].as_str().unwrap().contains("recomputed 0.25"));
}

#[test]
fn a_verifier_that_is_the_scout_is_not_independent() {
    let mut c = full();
    let scout = c["spec"]["validation"]["scout_actor"].clone();
    c["out"]["validation"]["verifier_actor"] = scout;
    let inv = pick(&project(&c, None, AT), "investigation_set");
    let e = inv["evidence"].as_array().unwrap().iter().find(|e| e["evidence_ref"]["id"] == "ev-verifier-independence").unwrap();
    assert_eq!(e["relation"], "contradicts");
    assert!(e["summary"].as_str().unwrap().contains("NOT independent"));
}

#[test]
fn diff_is_the_structural_change_the_compile_step_produced() {
    let evs = project(&full(), None, AT);
    let ev = evs.iter().find(|e| e.kind == "diff_set").unwrap();
    let d = &ev.data;
    assert_eq!(ev.entity_kind, "proposal");
    assert_eq!(ev.entity_id, d["proposal_id"].as_str().unwrap());
    let lines: Vec<String> = d["lines"].as_array().unwrap().iter().map(|l| format!("{} {}", l["op"].as_str().unwrap(), l["text"].as_str().unwrap())).collect();
    assert!(lines.contains(&"del   prompt:resumen_radicado@1".to_string()), "{lines:?}");
    assert!(lines.contains(&"add   prompt:resumen_radicado@2".to_string()), "{lines:?}");
    assert!(lines.contains(&"add   eval_suite:disputas-tarea-suite@2".to_string()), "an add op only adds: {lines:?}");
    assert!(!lines.iter().any(|l| l.starts_with("del") && l.contains("eval_suite")), "{lines:?}");
    assert!(lines.last().unwrap().contains("no text diff is shown"), "says what is not available");
    assert_eq!(d["proposal_id"], full()["out"]["arms"]["frozen"]["proposal_id"]);
}

#[test]
fn a_denied_compile_has_no_diff_no_gates_and_a_blocked_change_hypothesis() {
    let mut c = full();
    c["out"] = json!({"sensors": c["out"]["sensors"], "recompute": c["out"]["recompute"], "validation": c["out"]["validation"], "compile": {"status": "denied", "denied_reason": "kind_not_supported"}});
    let evs = project(&c, None, AT);
    assert!(evs.iter().all(|e| e.kind != "diff_set" && e.kind != "gates_set" && e.kind != "decision_set"), "{:?}", evs.iter().map(|e| &e.kind).collect::<Vec<_>>());
    let inv = pick(&evs, "investigation_set");
    assert_eq!(inv["hypotheses"][1]["verdict"], "blocked(kind_not_supported)");
    let alts = pick(&evs, "alternatives_set");
    assert!(alts["items"][1]["summary"].as_str().unwrap().contains("kind_not_supported"));
}

#[test]
fn gates_split_native_and_improvement_with_the_reason_the_improvement_failed() {
    let g = pick(&project(&full(), None, AT), "gates_set");
    assert_eq!(g["improvement"]["status"], "fail");
    assert_eq!(g["improvement"]["reason_code"], "no_structural_improvement");
    assert_eq!(g["native"]["status"], "pass");
    assert_eq!(g["native"]["report_ref"]["media_type"], "application/json");
    assert_eq!(g["native"]["report_ref"]["digest"].as_str().unwrap().len(), 64);
    assert_eq!(g["combined"]["decision"], "override_simulated_human");
    assert!(g["combined"]["reason_code"].as_str().unwrap().contains("no_structural_improvement"));
    let a = &g["attempts"][0];
    assert_eq!(a["improvement"]["status"], "fail");
    assert!(a["native_reason"].as_str().unwrap().contains("offline double"));
    assert_eq!(g["proposal_id"], full()["out"]["arms"]["frozen"]["proposal_id"]);
}

#[test]
fn the_decision_card_is_labelled_simulated_with_the_gate_state_it_was_taken_on() {
    let evs = project(&full(), None, AT);
    let ev = evs.iter().find(|e| e.kind == "decision_set").unwrap();
    let d = &ev.data;
    assert_eq!(ev.entity_id, d["decision_id"].as_str().unwrap());
    assert_eq!(d["available_commands"], json!([]), "nobody can act: the human is simulated");
    let c = &d["card"];
    assert_eq!(c["label"], "SIMULATED");
    assert_eq!(c["simulated"], true);
    assert_eq!(c["state"], "approved");
    assert_eq!(c["gate"]["improvement"]["status"], "fail");
    assert_eq!(c["override"]["label"], "human_override");
    assert_eq!(c["override"]["simulated"], true);
    assert!(c["reasons"].to_string().contains("SIMULATED human override") && c["reasons"].to_string().contains("not a quality claim"));
}

#[test]
fn a_blocked_approval_is_a_blocked_card_not_a_missing_one() {
    let mut c = full();
    c["out"].as_object_mut().unwrap().remove("authority");
    c["out"].as_object_mut().unwrap().remove("publish");
    let report = json!({"steps": [{"id": "approval", "status": "blocked(gate)"}]});
    let d = pick(&project(&c, Some(&report), AT), "decision_set");
    assert_eq!(d["card"]["state"], "blocked");
    assert!(d["card"]["reasons"][0].as_str().unwrap().contains("blocked(gate)"));
    assert!(project(&c, None, AT).iter().all(|e| e.kind != "decision_set"), "without authority or a blocked step nothing is decided");
}

#[test]
fn partial_payloads_fill_only_what_is_committed_and_the_projection_is_deterministic() {
    let mut c = full();
    let out = c["out"].as_object().unwrap().clone();
    c["out"] = json!({"sensors": out["sensors"]});
    assert!(project(&c, None, AT).is_empty(), "nothing but signals: no panel claims anything");
    c["out"] = json!({"sensors": out["sensors"], "recompute": out["recompute"], "validation": out["validation"]});
    let kinds: Vec<String> = project(&c, None, AT).iter().map(|e| e.kind.clone()).collect();
    assert_eq!(kinds, ["investigation_set"]);
    let data = |c: &Value| serde_json::to_string(&project(c, None, AT).iter().map(|e| &e.data).collect::<Vec<_>>()).unwrap();
    assert_eq!(data(&full()), data(&full()));
}

#[test]
fn an_approval_without_override_never_claims_both_gates_passed_unless_the_committed_gates_say_so() {
    let mut c = full();
    c["out"]["authority"].as_object_mut().unwrap().remove("override");
    let d = pick(&project(&c, None, AT), "decision_set");
    let text = d["card"]["reasons"].to_string();
    assert_eq!(d["card"]["gate"]["improvement"]["status"], "fail", "fixture: improvement failed");
    assert!(!text.contains("Both gates passed"), "reason invented a pass the committed gate does not carry: {text}");
    assert!(text.contains("SIMULATED"), "{text}");
}

fn limit_text(report: Option<&Value>) -> (String, String) {
    let evs = project(&full(), report, AT);
    let inv = pick(&evs, "investigation_set");
    let lim = inv["evidence"].as_array().unwrap().iter().find(|e| e["evidence_ref"]["id"] == "ev-limit-scripted-scout").expect("scout limit evidence").clone();
    (lim["summary"].as_str().unwrap().to_string(), inv["hypothesis"].as_str().unwrap().to_string())
}

fn scout_report(label: &str, provider: &str, outcome: &str, real: bool) -> Value {
    json!({"models": [{"role": "scout", "label": label, "model_id": provider.strip_prefix("gateway:").unwrap_or("m-1"), "outcome": outcome, "provider": provider, "real": real, "status": if real { "real" } else { "stand-in" }}]})
}

#[test]
fn scout_limit_for_a_scripted_model_says_scripted_and_no_model() {
    let (t, h) = limit_text(Some(&scout_report("scripted", "scripted", "answered", false)));
    assert!(t.contains("scripted") && t.contains("no model produced it"), "{t}");
    assert!(h.starts_with("Scripted scout claims"), "{h}");
}

#[test]
fn scout_limit_for_a_roleplay_replay_does_not_say_scripted_and_names_the_replay() {
    let (t, h) = limit_text(Some(&scout_report("roleplay", "agent-roleplay", "answered", false)));
    assert!(t.contains("roleplay") && t.contains("replay") && t.contains("not a real model"), "{t}");
    assert!(!t.contains("scripted value") && !t.contains("no model produced it"), "{t}");
    assert!(!h.contains("Scripted"), "{h}");
}

#[test]
fn scout_limit_for_a_gateway_answer_names_the_gateway_model_and_does_not_say_no_model() {
    let (t, h) = limit_text(Some(&scout_report("gateway", "gateway:claude-x", "answered", true)));
    assert!(t.contains("gateway") && t.contains("claude-x"), "{t}");
    assert!(!t.contains("scripted value") && !t.contains("no model produced it"), "{t}");
    assert!(!h.contains("Scripted"), "{h}");
}

#[test]
fn scout_limit_without_a_report_keeps_the_scripted_text() {
    let (t, _) = limit_text(None);
    assert!(t.contains("scripted") && t.contains("no model produced it"), "{t}");
}

// ---- W7: a signal that came from a real sensor run over a platform event package ----

fn source() -> Value {
    json!({"sensor": "rust-events", "data_mode": "platform", "data_origin": "platform_live", "adapter": "product-sqlite", "source_id": "platform:sim",
           "package": "pkg-0123456789abcdef", "events_read": 3000, "metric_id": "reassignment_rate.pt.web_chat", "cell": "pt/web_chat", "numerator": 54, "denominator": 321})
}

#[test]
fn a_report_with_a_source_shows_the_real_sensor_signal_and_stops_calling_the_sensor_a_stand_in_that_reads_no_data() {
    let report = json!({"models": [], "source": source()});
    let inv = pick(&project(&full(), Some(&report), AT), "investigation_set");
    let ev = |id: &str| inv["evidence"].as_array().unwrap().iter().find(|e| e["evidence_ref"]["id"] == id).cloned().unwrap_or_else(|| panic!("no {id}: {inv}"));
    let sig = ev("ev-source-signal");
    let text = sig["summary"].as_str().unwrap();
    assert!(text.contains("rust-events") && text.contains("pt/web_chat") && text.contains("54/321") && text.contains("pkg-0123456789abcdef"), "{text}");
    assert_eq!(sig["relation"], "supports");
    let limit = ev("ev-limit-sensor")["summary"].as_str().unwrap().to_string();
    assert!(limit.contains("fixed-output stand-in") && limit.contains("lab row"), "{limit}");
    assert!(!limit.contains("reads no data"), "the real sensor did read data: {limit}");
    let ids = evidence_ids(&inv);
    for h in inv["hypotheses"].as_array().unwrap() {
        for r in h["evidence_refs"].as_array().unwrap() {
            assert!(ids.contains(&r.as_str().unwrap().to_string()));
        }
    }
}

#[test]
fn without_a_source_the_sensor_limit_keeps_its_offline_wording() {
    let inv = pick(&project(&full(), Some(&json!({"models": []})), AT), "investigation_set");
    let text = inv.to_string();
    assert!(text.contains("it reads no data") && !text.contains("ev-source-signal"), "{text}");
}
