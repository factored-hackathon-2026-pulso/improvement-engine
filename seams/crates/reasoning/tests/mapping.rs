//! MAP1: the data-driven mapping table (ranked candidates with a justification, hypothesis label, human-owned findings) and the
//! per-candidate attempt of the pipeline.
mod common;
use common::*;
use reasoning::finding::Source;
use reasoning::mapping::{Caps, MAX_CANDIDATES, Mapped, Table, human_owned, map_finding};
use reasoning::pipeline::{Opts, candidate_plan, reason, reason_candidate};
use reasoning::testkit::FnPort;
use serde_json::{Value, json};
use std::cell::Cell;
use std::rc::Rc;

fn cell(metric: &str, reason: &str, channel: &str) -> reasoning::Finding {
    finding_of(signal(metric, json!({"reason_category": reason, "channel": channel}), stage(560, 1000, 0.17), stage(2800, 5000, 0.17)), Source::BankTreated)
}

fn bundled_json() -> Value {
    serde_json::from_str(include_str!("../fixtures/mapping_table.json")).unwrap()
}

#[test]
fn the_bundled_table_is_valid_and_every_candidate_is_grounded() {
    let t = Table::bundled();
    assert_eq!(t.label, "hypothesis_table");
    assert!(t.notice_es.contains("hipotesis") && t.notice_pt.contains("hipotese") && t.notice_en.contains("hypothesis"));
    assert!(t.provenance["sources"]["anatomy"].as_str().unwrap().contains("ARTIFACT_ANATOMY"));
    let catalog = cat();
    let suite_py = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../scripts/regression/build_suite.py")).expect("the suite generator is in the repo");
    for c in t.all_targets() {
        assert!(!c.justification.is_empty() && !c.evidence.is_empty(), "{}", c.target_ref);
        reasoning::clean_text(c.justification, 400).unwrap_or_else(|e| panic!("{}: {e}", c.target_ref));
        match c.kind {
            "patch" => {
                let art = catalog.get(&c.target_ref).unwrap_or_else(|| panic!("{} is not in the catalogue", c.target_ref));
                assert!(!art.locales.is_empty(), "{}", c.target_ref);
                assert!(catalog.agent(c.agent).is_some(), "{}: unknown agent {}", c.target_ref, c.agent);
            }
            _ => assert!(!c.slugs.is_empty() && catalog.agent(c.agent).is_some()),
        }
        // `proof_support` and `announceable_now` must agree with the real suite generator
        let has_generator = suite_py.contains(&format!("\"{}\":", c.target_ref));
        assert_eq!(c.proof_support != "none", has_generator, "{}: proof_support disagrees with scripts/regression/build_suite.py", c.target_ref);
        if c.announceable_now {
            assert!(has_generator && c.kind == "patch", "{} claims announceable_now without a generator", c.target_ref);
        }
    }
}

#[test]
fn a_complaint_cell_prefers_patches_of_existing_covering_agents_over_a_new_agent() {
    for ch in ["Phone", "Email", "App", "WhatsApp"] {
        let row = map_finding(&cell("M1", "Queja", ch)).expect("Queja maps");
        assert_eq!(row.id, "complaint_unresolved");
        assert_eq!(row.topic, "complaint_followup");
        assert!(row.covered_by.contains(&"consultas") && row.covered_by.contains(&"disputas"));
        assert_eq!(row.targets[0].target_ref, "template:t/estado_pqr", "{ch}");
        assert_eq!(row.targets[1].target_ref, "prompt:p/resumen_radicado", "{ch}");
        // the new agent is the LAST candidate, never first
        assert_eq!(row.targets.last().unwrap().kind, "new_agent");
        let tried: Vec<_> = row.ordered(Caps::default()).iter().map(|t| t.target_ref.clone()).collect();
        assert_eq!(tried, vec!["template:t/estado_pqr", "prompt:p/resumen_radicado"], "{ch}: at most {MAX_CANDIDATES} candidates, in rank order");
        assert_eq!(row.targets.iter().any(|t| t.target_ref == "prompt:p/copiloto"), ch == "Phone", "{ch}: the advisor copilot is a Phone candidate only");
        assert!(row.caveats.contains(&"mapping_is_a_hypothesis_of_where_to_intervene"));
    }
    assert_eq!(candidate_plan(&cell("M1", "Queja", "Phone"), Caps::default()), vec!["template:t/estado_pqr", "prompt:p/resumen_radicado"]);
}

#[test]
fn uncovered_topics_get_a_new_agent_first_and_the_advisor_copilot_second() {
    for (reason, slug) in [("Tecnico", "soporte-tecnico"), ("Comercial", "soporte-comercial"), ("Retencion", "soporte-retencion")] {
        let row = map_finding(&cell("M1", reason, "Phone")).unwrap();
        assert_eq!(row.id, "uncovered_reason");
        assert!(row.covered_by.is_empty(), "no agent covers {reason}");
        assert_eq!(row.targets[0].kind, "new_agent");
        assert!(row.targets[0].slugs.contains(&slug));
        assert_eq!(row.targets[1].target_ref, "prompt:p/copiloto");
        // a digital channel has no existing artifact on its path: new agent only
        assert_eq!(map_finding(&cell("M1", reason, "App")).unwrap().targets.len(), 1);
    }
    assert!(map_finding(&cell("M1", "Producto", "Phone")).is_none());
}

#[test]
fn what_is_announceable_now_is_tried_first_without_losing_the_priority_order_otherwise() {
    // hypothetical table: a new agent ranked first, an announceable patch second
    let mut j = bundled_json();
    let cands = &mut j["rows"][1]["candidates"];
    let mut patch = bundled_json()["rows"][0]["candidates"][0].clone();
    patch["rank"] = json!(2);
    cands[1] = patch;
    let t = Table::parse(&j).unwrap();
    let Mapped::Row(row) = t.classify(&cell("M1", "Tecnico", "Phone")) else { panic!("row") };
    assert_eq!(row.targets[0].kind, "new_agent");
    let order: Vec<_> = row.ordered(Caps::default()).iter().map(|t| t.target_ref.clone()).collect();
    assert_eq!(order, vec!["template:t/estado_pqr", "new_agent:consultas"], "the announceable patch jumps the queue");
    // with an admin credential for release settings both can be announced: table rank wins again
    let order: Vec<_> = row.ordered(Caps { new_agent_admin: true }).iter().map(|t| t.target_ref.clone()).collect();
    assert_eq!(order, vec!["new_agent:consultas", "template:t/estado_pqr"]);
}

#[test]
fn the_loader_refuses_what_must_not_be_representable() {
    let broken = |edit: &dyn Fn(&mut Value)| {
        let mut j = bundled_json();
        edit(&mut j);
        Table::parse(&j).expect_err("must be refused")
    };
    assert!(broken(&|j| j["rows"][0]["candidates"][0]["kind"] = json!("policy")).contains("not representable"));
    assert!(broken(&|j| {
        j["rows"][0]["candidates"][0]["target_ref"] = json!("new_agent:pulso-builder");
        j["rows"][0]["candidates"][0]["kind"] = json!("new_agent");
    })
    .contains("deny-listed"));
    assert!(broken(&|j| j["rows"][0]["candidates"][0]["target_ref"] = json!("template:t/release_settings")).contains("deny-listed"));
    assert!(broken(&|j| j["rows"][0]["candidates"][1]["rank"] = json!(7)).contains("ranks"));
    assert!(broken(&|j| j["rows"][0]["caveats"] = json!(["where_not_why"])).contains("hypothesis"));
    assert!(broken(&|j| j["rows"][0]["link_grade"] = json!("same_outcome_linked")).contains("link_grade"));
    assert!(broken(&|j| j["rows"][0]["candidates"][0]["mechanisms"] = json!(["invented"])).contains("mechanisms"));
    assert!(broken(&|j| j["rows"][0]["match"] = json!({})).contains("empty match"));
    assert!(broken(&|j| j["rows"][0]["candidates"][0]["proof_support"] = json!("trust_me")).contains("proof_support"));
    assert!(broken(&|j| j["rows"][0]["candidates"][4]["slugs"] = json!([])).contains("slugs"));
}

#[test]
fn human_owned_findings_are_never_mapped_to_an_artifact() {
    let amount = finding_of(signal("E2", json!({"reason_code": "policy:escalamiento-disputa-monto"}), stage(300, 1000, 0.1), stage(200, 700, 0.1)), Source::E0Treated);
    let h = human_owned(&amount).expect("the dispute amount policy is human owned");
    assert_eq!((h.id, h.owner), ("policy_dispute_amount", "riesgo"));
    assert!(h.note_es.contains("persona") && h.note_pt.contains("pessoa"));
    assert!(map_finding(&amount).is_none());
    // a Portuguese weakness is owned by the platform (calibration is es-only) even when an artifact row would also match
    let pt = finding_of(signal("M1", json!({"reason_category": "Queja", "channel": "Phone", "language": "pt"}), stage(560, 1000, 0.17), stage(2800, 5000, 0.17)), Source::BankTreated);
    assert_eq!(human_owned(&pt).unwrap().id, "calibration_pt_thresholds");
    assert!(map_finding(&pt).is_none() && candidate_plan(&pt, Caps::default()).is_empty());
}

#[test]
fn a_human_owned_finding_ends_human_owned_with_a_note_and_calls_no_model() {
    let amount = finding_of(signal("E2", json!({"reason_code": "policy:escalamiento-disputa-monto"}), stage(300, 1000, 0.1), stage(200, 700, 0.1)), Source::Synthetic);
    let n = Rc::new(Cell::new(0u32));
    let any = |c: Rc<Cell<u32>>| FnPort::scripted("x", count_calls(c, |_| panic!("no model may be called for a human-owned finding")));
    let p = ports(any(n.clone()), any(n.clone()), any(n.clone()));
    let r = reason(&cat(), &amount, &p, &opts());
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.stage.as_str()), ("human_owned", "policy_dispute_amount", "mapping"));
    assert_eq!(n.get(), 0);
    let h = r.human_owned.as_ref().unwrap();
    assert_eq!((h["owner"].as_str(), h["builder_proposal"].as_bool()), (Some("riesgo"), Some(false)));
    assert!(h["note"]["es"].as_str().unwrap().len() < 400 && r.compiled.is_none() && r.dossier.is_none());
}

fn estado_builder() -> impl Fn(&engine::models::ModelRequest) -> Result<Value, engine::models::ModelError> + 'static {
    |_| {
        Ok(json!({"proposal": {"kind": "patch", "target_ref": "template:t/estado_pqr", "rationale": "Say what the status is.", "expected_direction": "decrease",
            "patches": [{"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": "Ya consult\u{e9} tu PQR y su estado actual es {{ facts.pqr.value.status }}."},
                        {"locale": "pt", "anchor_id": "pt.a1", "op": "replace", "replacement": "J\u{e1} consultei sua solicita\u{e7}\u{e3}o e o estado atual \u{e9} {{ facts.pqr.value.status }}."}],
            "alternatives": alts(), "uncertainty": "Association only."}}))
    }
}

#[test]
fn one_attempt_sees_one_candidate_and_the_proposal_carries_the_hypothesis_label() {
    let f = cell("M1", "Queja", "Phone");
    let seen = Rc::new(Cell::new(0usize));
    let s2 = seen.clone();
    let scout = FnPort::scripted("s", move |req| {
        s2.set(req.payload["inputs"]["allowed_targets"].as_array().map_or(0, Vec::len));
        scout_ok(&f_clone(), "template:t/estado_pqr", "status_message_gap")(req)
    });
    let p = ports(scout, FnPort::scripted("v", verifier_ok("supported")), FnPort::scripted("b", estado_builder()));
    let r = reason_candidate(&cat(), &f, &p, &Opts { allow_derived_aggregates: true }, Some("template:t/estado_pqr"));
    assert_eq!(r.status, "proposed", "{}", r.detail);
    assert_eq!(seen.get(), 1, "the Scout sees exactly the candidate under attempt, not the whole row");
    let c = r.candidate.as_ref().unwrap();
    assert_eq!((c["rank"].as_u64(), c["candidates_total"].as_u64(), c["claim"].as_str()), (Some(1), Some(5), Some("hypothesis_of_where_to_intervene_not_a_cause")));
    assert!(c["justification"].as_str().unwrap().contains("static sentence"));
    let eff = &r.compiled.as_ref().unwrap()["expected_effect"]["mapping"];
    assert_eq!(eff["claim"], "hypothesis_of_where_to_intervene_not_a_cause");
    assert!(r.compiled.as_ref().unwrap()["changes"][0]["docs"]["description"].as_str().unwrap().contains("hypothesis of where to intervene, not a cause"));
    let es = r.dossier.as_ref().unwrap()["es"]["sections"]["risks"].as_str().unwrap().to_string();
    assert!(es.contains("Hipótesis de dónde intervenir, no una causa (candidato 1 de 5)"), "{es}");
    // a target outside the row is refused as a typed stop
    let bad = reason_candidate(&cat(), &f, &p, &Opts { allow_derived_aggregates: true }, Some("template:t/aclarar_problema"));
    assert_eq!((bad.status.as_str(), bad.reason.as_str()), ("blocked", "precondition_missing"));
}

fn f_clone() -> reasoning::Finding {
    cell("M1", "Queja", "Phone")
}

#[test]
fn candidate_lists_say_why_a_candidate_was_not_tried() {
    let row = map_finding(&cell("M1", "Queja", "Phone")).unwrap();
    let list = row.candidate_list(Caps::default(), &["template:t/estado_pqr".to_string()]);
    assert_eq!(list.len(), 5);
    assert_eq!(list[0]["tried"], true);
    assert_eq!(list[1]["not_tried_because"], "an_earlier_candidate_was_proven_or_the_finding_stopped");
    assert_eq!(list[2]["not_tried_because"], "over_the_candidate_cap");
    assert_eq!((list[2]["proof_support"].as_str(), list[2]["announceable_now"].as_bool()), (Some("none"), Some(false)));
    assert!(list.iter().all(|c| c["justification"].as_str().is_some_and(|s| !s.is_empty()) && c["evidence"].as_str().is_some()));
}
