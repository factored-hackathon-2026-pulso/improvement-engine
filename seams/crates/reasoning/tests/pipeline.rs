//! Scout -> Verifier -> Builder over scripted ports (offline). Every scripted answer is produced from the REQUEST, and the port runs
//! the same TPS scan as the real ones, so these tests also prove that the three payloads are treated, opaque and clean.
mod common;
use common::*;
use engine::models::ModelError;
use reasoning::finding::{Finding, Source};
use reasoning::pipeline::{Opts, reason, reason_report};
use reasoning::testkit::{FnPort, menu_of};
use serde_json::{Value, json};
use std::cell::Cell;
use std::rc::Rc;

fn tecnico_builder(slug: &'static str) -> impl Fn(&engine::models::ModelRequest) -> Result<Value, ModelError> + 'static {
    move |_| {
        Ok(json!({"proposal": {"kind": "new_agent", "target_ref": "new_agent:consultas", "agent_id": slug, "rationale": "A narrow intake for the uncovered topic.", "expected_direction": "decrease",
            "routing": {"summary_es": "Recibe problemas t\u{e9}cnicos de la aplicaci\u{f3}n y los pasa a una persona.", "summary_pt": "Recebe problemas t\u{e9}cnicos do aplicativo e os encaminha a uma pessoa.",
                        "examples_es": ["la app se cierra sola", "no puedo entrar a la aplicaci\u{f3}n"], "examples_pt": ["o aplicativo fecha sozinho", "n\u{e3}o consigo entrar no aplicativo"]},
            "intake": {"ask_es": "Cu\u{e9}ntame qu\u{e9} problema tienes con la aplicaci\u{f3}n.", "ask_pt": "Conte qual problema voc\u{ea} tem com o aplicativo.",
                       "notice_es": "Gracias, una persona del equipo te contactar\u{e1}.", "notice_pt": "Obrigado, uma pessoa da equipe vai falar com voc\u{ea}."},
            "alternatives": alts(), "uncertainty": "The data says where the problem is, not why."}}))
    }
}

fn happy_ports(f: &Finding) -> reasoning::pipeline::Ports {
    ports(
        FnPort::scripted("scripted-scout", scout_ok(f, "new_agent:consultas", "uncovered_topic")),
        FnPort::scripted("scripted-verifier", verifier_ok("supported")),
        FnPort::scripted("scripted-builder", tecnico_builder("soporte-tecnico")),
    )
}

#[test]
fn a_real_sensor_signal_becomes_an_opportunity_a_verification_and_a_new_agent_proposal() {
    let f = tecnico_finding();
    assert_eq!((f.metric.as_str(), f.dims["channel"].as_str()), ("M1", "Phone"));
    let r = reason(&cat(), &f, &happy_ports(&f), &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str()), ("proposed", "compiled"), "{:?}", r.detail);
    assert_eq!(r.opportunity.as_ref().unwrap()["target_ref"], "new_agent:consultas");
    assert_eq!(r.verification.as_ref().unwrap()["status"], "supported");
    let p = r.compiled.as_ref().unwrap();
    assert_eq!(p["kind"], "new_agent");
    assert_eq!(p["changes"][0]["content"]["id"], "soporte-tecnico");
    assert_eq!(p["expected_effect"]["evidence_ref"], f.evidence_ref().as_str());
    let rub = r.rubric.as_ref().unwrap();
    assert_eq!(rub["hard_gates"]["R4"], true);
    assert_eq!(rub["hard_gates"]["R11"], true);
    assert_eq!(rub["human_review"], "always (new agent)");
    assert_eq!(rub["band"], "revise", "a new agent is never auto-adequate and no suite is authored in this lane");
}

#[test]
fn every_scripted_call_is_labelled_scripted_and_listed_as_a_double() {
    let f = tecnico_finding();
    let r = reason(&cat(), &f, &happy_ports(&f), &opts());
    assert_eq!(r.calls.len(), 3);
    for c in &r.calls {
        assert_eq!((c["label"].as_str(), c["real"].as_bool(), c["status"].as_str()), (Some("scripted"), Some(false), Some("stand-in")));
    }
    let parts: Vec<&str> = r.doubles.iter().map(|d| d["part"].as_str().unwrap()).collect();
    assert_eq!(parts, ["model.scout", "model.verifier", "model.builder"]);
    assert_eq!(r.independence["level"], "other_model", "different model ids -> the verifier is a different model");
    assert_eq!(r.independence["verifier_separate_port"], true);
}

#[test]
fn the_verifier_never_sees_the_scouts_text() {
    let f = tecnico_finding();
    let seen = Rc::new(std::cell::RefCell::new(String::new()));
    let s2 = seen.clone();
    let verifier = FnPort::scripted("v", move |req| {
        *s2.borrow_mut() = format!("{}{}", req.system, req.payload);
        verifier_ok("supported")(req)
    });
    let p = ports(FnPort::scripted("s", scout_ok(&f, "new_agent:consultas", "uncovered_topic")), verifier, FnPort::scripted("b", tecnico_builder("soporte-tecnico")));
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!(r.status, "proposed");
    let text = seen.borrow().clone();
    for secret in ["no wording for this situation", "falsifier", "Thresholds and policies", "The gap is replicated"] {
        assert!(!text.contains(secret), "the verifier payload leaks {secret:?}");
    }
    assert!(text.contains("claimed_rate") && text.contains("\"w1\""), "it does see the claim and the evidence rows");
    assert!(text.contains("independent Verifier"));
}

#[test]
fn a_claim_that_does_not_recompute_stops_before_the_verifier_is_called() {
    let f = tecnico_finding();
    let calls = Rc::new(Cell::new(0));
    let c2 = calls.clone();
    let wrong = move |_: &engine::models::ModelRequest| -> Result<Value, ModelError> {
        Ok(json!({"opportunity": {"id": "h_1", "target_ref": "new_agent:consultas", "mechanism_class": "uncovered_topic", "claimed_rate": 0.31, "hypothesis": "No specialist exists.",
            "falsifiers": ["Same rate elsewhere."], "alternatives": alts()}}))
    };
    let p = ports(FnPort::scripted("s", wrong), FnPort::scripted("v", count_calls(c2, verifier_ok("supported"))), FnPort::scripted("b", tecnico_builder("soporte-tecnico")));
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.stage.as_str()), ("blocked", "claim_not_corroborated", "recompute"));
    assert_eq!(calls.get(), 0);
    assert_eq!(r.calls.len(), 1);
}

#[test]
fn an_independent_refutation_stops_before_the_builder_and_a_weakening_asks_for_a_human() {
    let f = tecnico_finding();
    let builder_calls = Rc::new(Cell::new(0));
    let b2 = builder_calls.clone();
    let p = ports(FnPort::scripted("s", scout_ok(&f, "new_agent:consultas", "uncovered_topic")), FnPort::scripted("v", verifier_ok("refuted")), FnPort::scripted("b", count_calls(b2, tecnico_builder("soporte-tecnico"))));
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str()), ("blocked", "verifier_refuted"));
    assert_eq!(builder_calls.get(), 0);

    let p = ports(FnPort::scripted("s", scout_ok(&f, "new_agent:consultas", "uncovered_topic")), FnPort::scripted("v", verifier_ok("weakened")), FnPort::scripted("b", tecnico_builder("soporte-tecnico")));
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!(r.status, "proposed");
    assert_eq!(r.verification.as_ref().unwrap()["human_review_required"], true);
}

#[test]
fn a_deterministic_failure_refutes_whatever_the_model_says() {
    // holdout rate below its baseline: the replication check fails in code even though the scripted verifier says supported
    let f = finding_of(
        signal("M1", json!({"reason_category": "Tecnico", "channel": "Phone"}), stage(2700, 6000, 0.20), stage(300, 4000, 0.20)),
        Source::Synthetic,
    );
    let p = ports(FnPort::scripted("s", scout_ok(&f, "new_agent:consultas", "uncovered_topic")), FnPort::scripted("v", verifier_ok("supported")), FnPort::scripted("b", tecnico_builder("soporte-tecnico")));
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str()), ("blocked", "verifier_refuted"));
    assert_eq!(r.verification.as_ref().unwrap()["status"], "refuted");
}

#[test]
fn invalid_scout_answers_are_typed_stops_never_a_fallback() {
    let f = tecnico_finding();
    let stop = |scout: Box<dyn Fn(&engine::models::ModelRequest) -> Result<Value, ModelError>>| {
        let p = ports(FnPort::scripted("s", scout), FnPort::scripted("v", verifier_ok("supported")), FnPort::scripted("b", tecnico_builder("soporte-tecnico")));
        let r = reason(&cat(), &f, &p, &opts());
        (r.status, r.reason, r.stage, r.calls.len())
    };
    // a target outside the mapping row (a policy, a tool, a protected agent): refused by construction
    let bad_target = |t: &'static str| -> Box<dyn Fn(&engine::models::ModelRequest) -> Result<Value, ModelError>> {
        Box::new(move |_| Ok(json!({"opportunity": {"id": "h_1", "target_ref": t, "mechanism_class": "uncovered_topic", "claimed_rate": 0.45, "hypothesis": "x y z", "falsifiers": ["f"], "alternatives": alts()}})))
    };
    for t in ["policy:escalamiento-disputa-monto", "agent:constructor-chat", "template:t/estado_pqr", "tool:radicar_pqr"] {
        assert_eq!(stop(bad_target(t)), ("blocked".into(), "model_invalid".into(), "scout".into(), 1), "{t}");
    }
    assert_eq!(stop(Box::new(|_| Err(ModelError::Unavailable("gateway_unreachable".into())))).1, "model_unavailable");
    assert_eq!(stop(Box::new(|_| Err(ModelError::Refused("tps".into())))).1, "model_refused");
    assert_eq!(stop(Box::new(|_| Ok(json!({"opportunity": {"id": "h_1"}})))).1, "model_invalid");
    // free text with a long digit run or an email is refused at parse time
    let leaky = Box::new(|_: &engine::models::ModelRequest| Ok(json!({"opportunity": {"id": "h_1", "target_ref": "new_agent:consultas", "mechanism_class": "uncovered_topic", "claimed_rate": 0.45,
        "hypothesis": "Write to ayuda@banco.example about case 123456.", "falsifiers": ["f"], "alternatives": alts()}})));
    assert_eq!(stop(leaky).1, "model_invalid");
}

#[test]
fn a_denied_compile_is_a_typed_stop_with_the_compiler_reason() {
    let f = tecnico_finding();
    let p = ports(FnPort::scripted("s", scout_ok(&f, "new_agent:consultas", "uncovered_topic")), FnPort::scripted("v", verifier_ok("supported")), FnPort::scripted("b", tecnico_builder("soporte-pagos")));
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.stage.as_str()), ("blocked", "compile_denied:slug_not_allowed", "compile"));
    assert!(r.compiled.is_none());
}

#[test]
fn a_finding_without_a_mapping_is_unlinked_and_makes_no_model_call() {
    let f = finding_of(signal("M6", json!({"channel": "Phone"}), stage(300, 1000, 0.1), stage(200, 700, 0.1)), Source::Synthetic);
    let r = reason(&cat(), &f, &happy_ports(&f), &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.calls.len()), ("unlinked", "no_mapping", 0));
}

#[test]
fn derived_aggregates_reach_a_real_hosted_model_only_with_the_explicit_opt_in() {
    let f = copilot_finding(Source::E0Treated);
    let calls = Rc::new(Cell::new(0));
    let mk = |c: Rc<Cell<u32>>| {
        let f2 = f.clone();
        ports(
            FnPort::pretending_gateway("real-model-a", count_calls(c.clone(), scout_ok(&f2, "prompt:p/copiloto", "repeated_lookup"))),
            FnPort::pretending_gateway("real-model-b", count_calls(c.clone(), verifier_ok("supported"))),
            FnPort::pretending_gateway("real-model-a", count_calls(c, copilot_builder())),
        )
    };
    let r = reason(&cat(), &f, &mk(calls.clone()), &Opts::default());
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.stage.as_str()), ("blocked", "derived_data_opt_in_required", "policy"));
    assert_eq!(calls.get(), 0, "nothing was sent");
    let r = reason(&cat(), &f, &mk(calls.clone()), &Opts { allow_derived_aggregates: true });
    assert_eq!(r.status, "proposed", "{}", r.detail);
    assert_eq!(calls.get(), 3);
    // answered gateway-labelled calls are real, and then doubles[] is empty
    assert!(r.calls.iter().all(|c| c["real"] == true) && r.doubles.is_empty());
    // synthetic data needs no flag even for a real model, and scripted ports never need one
    let s = copilot_finding(Source::Synthetic);
    let ps = ports(FnPort::pretending_gateway("m", scout_ok(&s, "prompt:p/copiloto", "repeated_lookup")), FnPort::pretending_gateway("n", verifier_ok("supported")), FnPort::pretending_gateway("m", copilot_builder()));
    assert_eq!(reason(&cat(), &s, &ps, &Opts::default()).status, "proposed");
    let scripted = ports(FnPort::scripted("s", scout_ok(&f, "prompt:p/copiloto", "repeated_lookup")), FnPort::scripted("v", verifier_ok("supported")), FnPort::scripted("b", copilot_builder()));
    assert_eq!(reason(&cat(), &f, &scripted, &Opts::default()).status, "proposed");
}

fn copilot_builder() -> impl Fn(&engine::models::ModelRequest) -> Result<Value, ModelError> + 'static {
    |req| {
        let (es, pt) = (anchor_containing(req, "es", "Elige la tool seg"), anchor_containing(req, "pt", "Escolha a tool"));
        assert!(menu_of(req).iter().all(|(_, t)| !t.contains("datos_no_confiables")), "the menu has no protected clause");
        Ok(json!({"proposal": {"kind": "patch", "target_ref": "prompt:p/copiloto", "rationale": "Answer the repeated lookup in the first steps.", "expected_direction": "decrease",
            "patches": [{"locale": "es", "anchor_id": es, "op": "insert_after", "replacement": "Si el pedido es sobre cargos de una disputa, lee leer_movimientos y leer_pqr_cliente en tus dos primeros pasos."},
                        {"locale": "pt", "anchor_id": pt, "op": "insert_after", "replacement": "Se o pedido for sobre cobran\u{e7}as de uma disputa, leia leer_movimientos e leer_pqr_cliente nos dois primeiros passos."}],
            "alternatives": alts(), "uncertainty": "E0 is generated data and the advisor agent cannot be evaluated natively."}}))
    }
}

#[test]
fn the_copilot_prompt_proposal_scores_adequate_pending_the_suite() {
    let f = copilot_finding(Source::Synthetic);
    let p = ports(FnPort::scripted("s", scout_ok(&f, "prompt:p/copiloto", "repeated_lookup")), FnPort::scripted("v", verifier_ok("supported")), FnPort::scripted("b", copilot_builder()));
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!(r.status, "proposed", "{}", r.detail);
    let rub = r.rubric.as_ref().unwrap();
    assert_eq!(rub["band"], "adequate_pending_suite");
    assert_eq!(rub["total"], 23);
    assert_eq!(rub["hard_gates"]["R6b"], "not_evaluated");
    assert!(r.compiled.as_ref().unwrap()["cascade"].as_array().unwrap().iter().any(|c| c == "flow:asistir@1.0.1"));
}

#[test]
fn the_run_report_counts_outcomes_and_lists_every_double() {
    let sensor = synthetic_cells_report();
    let (found, skipped) = Finding::from_report(&sensor, Source::Synthetic).unwrap();
    assert!(!found.is_empty());
    let f = tecnico_finding();
    let p = happy_ports(&f);
    let rep = reason_report(&cat(), &sensor, Source::Synthetic, &p, &opts()).unwrap();
    assert_eq!(rep["sensor_semantics"], "claude-standin");
    assert_eq!(rep["baseline"]["label"], "fixture-baseline");
    assert_eq!(rep["quality_claims"], "forbidden");
    assert_eq!(rep["summary"]["corroborated"].as_u64().unwrap() as usize, found.len());
    assert_eq!(rep["summary"]["skipped_not_corroborated"].as_u64().unwrap() as usize, skipped.len());
    assert_eq!(rep["summary"]["proposed"], 1);
    assert_eq!(rep["doubles"].as_array().unwrap().len(), 3);
    // aggregates only: nothing in the report looks like a row, an id of a customer or an email
    let s = rep.to_string();
    assert!(!reasoning::email_like(&s) && !s.contains("customer_id"));
}

#[test]
fn a_level_risk_signal_never_becomes_a_contrast_finding() {
    let mut sensor = synthetic_cells_report();
    let (before, _) = Finding::from_report(&sensor, Source::Synthetic).unwrap();
    let risk = json!({"metric": "M8", "type": "level_risk", "class": "risk", "dims": {}, "status": "corroborated", "reason": "replicated_in_holdout",
        "direction": "up", "claim": "association",
        "discovery": {"numerator": 50, "denominator": 100, "rate": 0.5, "baseline_rate": 0.1, "diff": 0.4},
        "holdout": {"numerator": 50, "denominator": 100, "rate": 0.5, "baseline_rate": 0.1, "diff": 0.4}});
    sensor["signals"].as_array_mut().unwrap().push(risk);
    let (after, skipped) = Finding::from_report(&sensor, Source::Synthetic).unwrap();
    assert_eq!(after.len(), before.len());
    assert!(skipped.iter().any(|s| s.metric == "M8" && s.status == "level_risk"));
}
