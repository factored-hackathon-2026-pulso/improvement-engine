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
    assert_eq!(r.independence["level"], "other_family", "different vendors -> other family");
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
        // a rejected answer is retried twice with the problem fed back (3 calls), then it is a typed stop
        assert_eq!(stop(bad_target(t)), ("blocked".into(), "model_invalid".into(), "scout".into(), 3), "{t}");
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
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.calls.len()), ("unlinked", "dependency_metric", 0));
    assert!(r.detail.contains("M6") && r.detail.contains("M1"), "{}", r.detail);
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
fn independence_level_names_the_vendor_relation() {
    use reasoning::pipeline::independence_level as l;
    assert_eq!(l("xiaomi/mimo-v2.6-flash", "xiaomi/mimo-v2.6-pro"), "same_family_other_tier");
    assert_eq!(l("xiaomi/mimo-v2.6-flash", "z-ai/glm-5.3-flash"), "other_family");
    assert_eq!(l("xiaomi/mimo-v2.6-flash", "xiaomi/mimo-v2.6-flash"), "separate_prompt_and_context_only");
}

#[test]
fn a_cell_below_its_reference_is_not_an_opportunity_and_no_model_is_called() {
    let mut f = tecnico_finding();
    f.direction = "down".into();
    let calls = Rc::new(Cell::new(0u32));
    let c2 = calls.clone();
    let p = ports(
        FnPort::scripted("s", count_calls(c2.clone(), scout_ok(&f, "new_agent:consultas", "uncovered_topic"))),
        FnPort::scripted("v", count_calls(c2.clone(), verifier_ok("supported"))),
        FnPort::scripted("b", count_calls(c2, |_| Err(ModelError::Invalid("must not be called".into())))),
    );
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.stage.as_str()), ("no_change", "better_than_reference", "direction"));
    assert_eq!(calls.get(), 0);
}


fn lookup_scripted(f: &Finding) -> FnPort {
    FnPort::scripted("scripted-scout", scout_ok(f, "new_agent:consultas", "uncovered_topic"))
}

#[test]
fn an_unlinked_finding_names_its_reason_and_is_not_a_failure() {
    use reasoning::mapping::unlinked_reason;
    let m = |metric: &str| finding_of(signal(metric, json!({"channel": "Phone"}), stage(300, 1000, 0.1), stage(200, 700, 0.1)), Source::Synthetic);
    for (metric, code) in [("M6", "dependency_metric"), ("M6L", "dependency_metric"), ("M10", "dependent_on_m1"), ("M8", "level_risk_human_owned"), ("M99", "no_mapping")] {
        assert_eq!(unlinked_reason(&m(metric)).0, code, "{metric}");
    }
    let sensor = synthetic_cells_report();
    let f = tecnico_finding();
    let rep = reason_report(&cat(), &sensor, Source::Synthetic, &happy_ports(&f), &opts()).unwrap();
    assert!(rep["summary"]["unlinked_by_reason"].is_object());
    assert_eq!(rep["summary"]["failed"], rep["summary"]["blocked"], "an unlinked or no_change finding is never counted as a failure");
}

#[test]
fn a_rejected_answer_is_retried_with_the_problem_fed_back_and_the_second_answer_wins() {
    let f = tecnico_finding();
    let seen: Rc<std::cell::RefCell<Vec<Option<String>>>> = Rc::new(std::cell::RefCell::new(vec![]));
    let n = Rc::new(Cell::new(0u32));
    let (seen2, n2) = (seen.clone(), n.clone());
    let good = scout_ok(&f, "new_agent:consultas", "uncovered_topic");
    let scout = FnPort::scripted("scripted-scout", move |req| {
        n2.set(n2.get() + 1);
        seen2.borrow_mut().push(req.payload["feedback"].as_str().map(str::to_string));
        if n2.get() == 1 { Ok(json!({"opportunity": {"id": "h_1"}})) } else { good(req) }
    });
    let p = ports(scout, FnPort::scripted("v", verifier_ok("supported")), FnPort::scripted("b", tecnico_builder("soporte-tecnico")));
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!((r.status.as_str(), n.get()), ("proposed", 2), "{}", r.detail);
    let seen = seen.borrow();
    assert!(seen[0].is_none(), "the first attempt has no feedback");
    let fb = seen[1].as_ref().expect("the retry carries the problem");
    assert!(fb.contains("rejected") && fb.contains("ONE JSON object"), "{fb}");
    assert_eq!(r.metering["attempts"]["scout"], 2);
    // a third failure is final: bounded retries, never a loop
    let always_bad = FnPort::scripted("scripted-scout", |_| Ok(json!({"opportunity": {"id": "h_1"}})));
    let p = ports(always_bad, FnPort::scripted("v", verifier_ok("supported")), FnPort::scripted("b", tecnico_builder("soporte-tecnico")));
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.calls.len()), ("blocked", "model_invalid", 3));
}

#[test]
fn the_builders_direction_wording_target_ref_and_missing_direction_never_block_a_proposal() {
    let f = tecnico_finding();
    for patch in [json!({"expected_direction": "increase"}), json!({"expected_direction": "banana", "target_ref": "new_agent:soporte-tecnico"}), json!({"remove": "expected_direction"})] {
        let b = move |req: &engine::models::ModelRequest| {
            let mut a = tecnico_builder("soporte-tecnico")(req)?;
            for (k, v) in patch.as_object().unwrap() {
                if k == "remove" {
                    a["proposal"].as_object_mut().unwrap().remove(v.as_str().unwrap());
                } else {
                    a["proposal"][k] = v.clone();
                }
            }
            Ok(a)
        };
        let p = ports(lookup_scripted(&f), FnPort::scripted("v", verifier_ok("supported")), FnPort::scripted("b", b));
        let r = reason(&cat(), &f, &p, &opts());
        assert_eq!((r.status.as_str(), r.reason.as_str()), ("proposed", "compiled"), "{}", r.detail);
        assert_eq!(r.compiled.as_ref().unwrap()["expected_effect"]["direction"], "decrease");
        assert_eq!(r.metering["attempts"]["builder"], 1, "no retry is spent on wording the engine derives itself");
    }
}

#[test]
fn a_compile_denial_is_fed_back_to_the_builder_and_a_corrected_answer_compiles() {
    let f = tecnico_finding();
    let n = Rc::new(Cell::new(0u32));
    let n2 = n.clone();
    let b = move |req: &engine::models::ModelRequest| {
        n2.set(n2.get() + 1);
        if n2.get() == 1 {
            tecnico_builder("soporte-pagos")(req) // slug not allowed -> compile_denied:slug_not_allowed
        } else {
            let fb = req.payload["feedback"].as_str().unwrap_or("");
            assert!(fb.contains("compile") || fb.contains("slug"), "{fb}");
            tecnico_builder("soporte-tecnico")(req)
        }
    };
    let p = ports(lookup_scripted(&f), FnPort::scripted("v", verifier_ok("supported")), FnPort::scripted("b", b));
    let r = reason(&cat(), &f, &p, &opts());
    assert_eq!((r.status.as_str(), n.get()), ("proposed", 2), "{}", r.detail);
}

#[test]
fn the_stronger_builder_tier_is_tried_only_after_the_primary_fails_and_is_named_in_the_outcome() {
    let f = tecnico_finding();
    let mk = |primary: FnPort, esc: Option<FnPort>| {
        let mut p = ports(lookup_scripted(&f), FnPort::scripted("v", verifier_ok("supported")), primary);
        p.builder_escalation = esc.map(|e| Rc::new(e) as Rc<dyn engine::models::ModelPort>);
        p
    };
    // the primary (flash) succeeds: the escalation is never called
    let esc_calls = Rc::new(Cell::new(0u32));
    let ec = esc_calls.clone();
    let esc = FnPort::scripted("xiaomi/mimo-v2.6-pro", move |r| {
        ec.set(ec.get() + 1);
        tecnico_builder("soporte-tecnico")(r)
    });
    let r = reason(&cat(), &f, &mk(FnPort::scripted("xiaomi/mimo-v2.6-flash", tecnico_builder("soporte-tecnico")), Some(esc)), &opts());
    assert_eq!((r.status.as_str(), esc_calls.get()), ("proposed", 0));
    assert_eq!((r.metering["builder"]["tier"].as_str(), r.metering["builder"]["escalated"].as_bool()), (Some("flash"), Some(false)));
    // the primary keeps failing (3 attempts): the pro tier answers and the outcome says so
    let esc = FnPort::scripted("xiaomi/mimo-v2.6-pro", tecnico_builder("soporte-tecnico"));
    let r = reason(&cat(), &f, &mk(FnPort::scripted("xiaomi/mimo-v2.6-flash", tecnico_builder("soporte-pagos")), Some(esc)), &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str()), ("proposed", "compiled"), "{}", r.detail);
    assert_eq!((r.metering["builder"]["tier"].as_str(), r.metering["builder"]["escalated"].as_bool()), (Some("pro"), Some(true)));
    assert_eq!((r.metering["attempts"]["builder"].as_u64(), r.metering["attempts"]["builder_escalation"].as_u64()), (Some(3), Some(1)));
    let models: Vec<&str> = r.calls.iter().map(|c| c["model_id"].as_str().unwrap()).collect();
    assert_eq!(models.iter().filter(|m| m.ends_with("-pro")).count(), 1, "{models:?}");
    // no escalation configured: the failure is final and typed
    let r = reason(&cat(), &f, &mk(FnPort::scripted("xiaomi/mimo-v2.6-flash", tecnico_builder("soporte-pagos")), None), &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str()), ("blocked", "compile_denied:slug_not_allowed"));
    assert_eq!(r.metering["builder"]["tier"], "flash");
}

#[test]
fn tier_labels_come_from_the_model_id() {
    use reasoning::pipeline::tier_of;
    assert_eq!((tier_of("xiaomi/mimo-v2.6-flash"), tier_of("xiaomi/mimo-v2.6-pro"), tier_of("deepseek/deepseek-v4.1-flash"), tier_of("z-ai/glm-5.3-flash"), tier_of("scripted-v1")), ("flash", "pro", "flash", "flash", "other"));
}

#[test]
fn every_model_call_runs_inside_its_stage_and_attempt_of_the_story_so_the_gateway_gets_the_stage_span() {
    use engine::trace::{self, TraceCtx};
    let f = tecnico_finding();
    let seen: Rc<std::cell::RefCell<Vec<(String, u32)>>> = Rc::new(std::cell::RefCell::new(vec![]));
    let note = |seen: &Rc<std::cell::RefCell<Vec<(String, u32)>>>| {
        let c = trace::current().expect("a story scope");
        seen.borrow_mut().push((c.stage.unwrap(), c.attempt));
    };
    let (s1, s2, s3) = (seen.clone(), seen.clone(), seen.clone());
    let (scout, verifier, builder) = (scout_ok(&f, "new_agent:consultas", "uncovered_topic"), verifier_ok("supported"), tecnico_builder("soporte-tecnico"));
    let first_builder_try = Cell::new(true);
    let ports = ports(
        FnPort::scripted("scripted-scout", move |r| {
            note(&s1);
            scout(r)
        }),
        FnPort::scripted("scripted-verifier", move |r| {
            note(&s2);
            verifier(r)
        }),
        FnPort::scripted("scripted-builder", move |r| {
            note(&s3);
            // the first builder answer is unusable: the retry is attempt 2 of the same stage
            if first_builder_try.replace(false) { Ok(json!({"nonsense": true})) } else { builder(r) }
        }),
    );
    let _story = trace::enter(TraceCtx { finding_key: f.evidence_ref(), run_id: "value-loop-j1".into(), ..Default::default() });
    let r = reason(&cat(), &f, &ports, &opts());
    assert_eq!(r.status, "proposed", "{:?}", r.detail);
    let got: Vec<(String, u32)> = seen.borrow().clone();
    assert_eq!(got, vec![("scout".to_string(), 1), ("verifier".to_string(), 1), ("builder".to_string(), 1), ("builder".to_string(), 2)]);
}
