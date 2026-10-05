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
            "link_tool" => {
                let tool = c.params["tool"].as_str().expect("link_tool carries params.tool");
                assert!(c.target_ref.ends_with(&format!("/{tool}")) && catalog.tool_def(tool).is_some() && catalog.agent(c.agent).is_some(), "{}", c.target_ref);
            }
            "flow_edit" => {
                // FLOW1: the flow is the entry flow of the agent, and every preset compiles somewhere on the real graph
                let (flow, op) = (c.params["flow"].as_str().unwrap(), c.params["op"].as_str().unwrap());
                assert_eq!(c.target_ref, format!("flow_edit:{flow}/{op}"));
                let agent = catalog.agent(c.agent).unwrap_or_else(|| panic!("{}: unknown agent {}", c.target_ref, c.agent));
                assert_eq!(reasoning::art2::ref_id(&agent["entry_flow"]), flow, "{}", c.target_ref);
                let f = catalog.flow(flow).unwrap_or_else(|| panic!("{flow} is not in the catalogue"));
                for pj in c.params["presets"].as_array().unwrap() {
                    let preset = reasoning::flow_edits::Preset::from_json(pj).unwrap();
                    let ok = reasoning::flow_edits::menu(op, f).iter().any(|m| reasoning::flow_edits::compile_flow_edit(f, agent, op, m["position_id"].as_str().unwrap(), &preset, &|_| false).is_ok());
                    assert!(ok, "{}: preset {} compiles on no position of the real flow", c.target_ref, preset.id);
                }
            }
            _ => assert!(!c.slugs.is_empty() && catalog.agent(c.agent).is_some()),
        }
        // `proof_support` and `announceable_now` must agree with the real suite generator
        let key = if c.kind == "flow_edit" { format!("flow_{}", c.proof_support.trim_start_matches("suite:flow_")) } else { c.target_ref.clone() };
        let has_generator = suite_py.contains(&format!("\"{key}\":"));
        assert_eq!(c.proof_support != "none", has_generator, "{}: proof_support disagrees with scripts/regression/build_suite.py", c.target_ref);
        if c.announceable_now {
            assert!(has_generator && (c.kind == "patch" || c.kind == "link_tool" || c.kind == "flow_edit"), "{} claims announceable_now without a generator", c.target_ref);
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
        let pos = |k: &str| row.targets.iter().position(|t| t.kind == k).unwrap();
        assert!(pos("new_agent") < pos("flow_edit"), "the flow edits were appended after the new agent: no existing order changed");
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
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.stage.as_str()), ("needs_owner_ack", "needs_owner_ack", "compile"));
    assert_eq!(n.get(), 0);
    let h = r.human_owned.as_ref().unwrap();
    assert_eq!((h["owner"].as_str(), h["builder_proposal"].as_bool()), (Some("riesgo"), Some(false)));
    assert!(h["note"]["es"].as_str().unwrap().len() < 400 && r.dossier.is_none());
    // ART2: a policy finding is a hypothesis with the boundary evidence; the tighten-only draft is compiled deterministically and waits for the owner
    assert_eq!(h["policy_hypothesis"]["boundary_guards"], json!([249.0, 250.0, 251.0, 499.0, 500.0, 501.0]));
    assert!(h["policy_hypothesis"]["draft"].is_null());
    let c = r.compiled_raw.as_ref().expect("a tighten-only draft");
    assert_eq!((c.kind.as_str(), c.target_ref.as_str(), c.agent_id.as_str()), ("tighten_policy", "policy:escalamiento-disputa-monto", "disputas"));
    assert_eq!(c.changes[0]["content"]["expr"], json!({">": [{"var": "facts.monto_usd.value"}, 250.0]}));
    assert_eq!(c.changes[0]["content"]["owner"], "riesgo");
    assert!(c.changes[0]["docs"]["changelog"].as_str().unwrap().contains("owner_ack required") && c.human_items[0].contains("owner_ack required"));
    assert_eq!(c.cascade, vec!["flow:disputa-cargo@1.0.1", "agent:disputas@1.0.1"]);
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
    assert_eq!((c["rank"].as_u64(), c["candidates_total"].as_u64(), c["claim"].as_str()), (Some(1), Some(7), Some("hypothesis_of_where_to_intervene_not_a_cause")));
    assert!(c["justification"].as_str().unwrap().contains("static sentence"));
    let eff = &r.compiled.as_ref().unwrap()["expected_effect"]["mapping"];
    assert_eq!(eff["claim"], "hypothesis_of_where_to_intervene_not_a_cause");
    assert!(r.compiled.as_ref().unwrap()["changes"][0]["docs"]["description"].as_str().unwrap().contains("hypothesis of where to intervene, not a cause"));
    let es = r.dossier.as_ref().unwrap()["es"]["sections"]["risks"].as_str().unwrap().to_string();
    assert!(es.contains("Hipótesis de dónde intervenir, no una causa (candidato 1 de 7)"), "{es}");
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
    assert_eq!(list.len(), 7);
    assert_eq!(list[0]["tried"], true);
    assert_eq!(list[1]["not_tried_because"], "an_earlier_candidate_was_proven_or_the_finding_stopped");
    assert_eq!(list[2]["not_tried_because"], "over_the_candidate_cap");
    assert_eq!((list[2]["proof_support"].as_str(), list[2]["announceable_now"].as_bool()), (Some("none"), Some(false)));
    assert!(list.iter().all(|c| c["justification"].as_str().is_some_and(|s| !s.is_empty()) && c["evidence"].as_str().is_some()));
}

// ---- EVT1: platform event families (P_* metrics) --------------------------------------------------------------------------------

fn pcell(metric: &str, dims: Value) -> reasoning::Finding {
    finding_of(signal(metric, dims, stage(750, 1000, 0.40), stage(3000, 4000, 0.40)), Source::Synthetic)
}

#[test]
fn copilot_low_acceptance_by_case_type_maps_to_a_prompt_patch_of_copiloto_asesor() {
    let f = pcell("P_DRAFT_REJECT", json!({"case_type": "service_quality", "channel": "app_chat"}));
    let row = map_finding(&f).expect("a row for low draft acceptance");
    assert_eq!(row.id, "copilot_low_acceptance");
    assert_eq!(row.link_grade, "mechanism_proxy");
    assert_eq!(row.targets[0].target_ref, "prompt:p/copiloto");
    assert_eq!((row.targets[0].kind, row.targets[0].agent), ("patch", "copiloto-asesor"));
    assert!(row.caveats.contains(&"mapping_is_a_hypothesis_of_where_to_intervene") && row.caveats.contains(&"synthetic_or_aggregate_association_not_a_cause"));
    assert!(row.targets.iter().all(|t| t.proof_support == "none" && !t.announceable_now), "no suite generator exists for the copilot prompt");
}

#[test]
fn a_release_level_acceptance_drop_is_a_release_regression_row_on_the_same_prompt() {
    let f = pcell("P_DRAFT_REJECT", json!({"release": "rel-b", "agent": "copiloto-asesor@1.0.0"}));
    let row = map_finding(&f).expect("row");
    assert_eq!(row.id, "copilot_release_regression");
    assert_eq!(row.targets[0].target_ref, "prompt:p/copiloto");
}

#[test]
fn heavy_edits_none_suggestions_and_tool_mix_each_have_a_row_with_their_own_mechanism() {
    let cases = [
        ("P_DRAFT_HEAVY_EDIT", json!({"case_type": "unrecognized_charge", "channel": "phone_inbound"}), "copilot_heavy_edits", "wording"),
        ("P_SUGG_NONE", json!({"case_type": "app_issue", "channel": "web_chat"}), "copilot_suggestion_none", "uncovered_topic"),
        ("P_TOOL_USE", json!({"case_type": "undue_charge", "tool": "consultar_cargos"}), "copilot_tool_mix", "repeated_lookup"),
    ];
    for (metric, dims, id, mech) in cases {
        let row = map_finding(&pcell(metric, dims)).unwrap_or_else(|| panic!("{metric}"));
        assert_eq!(row.id, id);
        assert_eq!(row.targets[0].target_ref, "prompt:p/copiloto");
        assert!(row.targets[0].mechanisms.contains(&mech), "{metric}: {:?}", row.targets[0].mechanisms);
    }
    // suggestion none says so: the alternative (a tool link or a knowledge source) is a human decision, not an engine target
    let none = map_finding(&pcell("P_SUGG_NONE", json!({"case_type": "app_issue", "channel": "web_chat"}))).unwrap();
    assert!(none.caveats.contains(&"tool_link_or_knowledge_source_is_a_human_decision"));
}

#[test]
fn reassigned_case_types_and_copilot_failures_are_human_owned_the_engine_never_acts() {
    let r = pcell("P_TYPE_REASSIGN", json!({"case_type": "undue_charge", "channel": "app_chat"}));
    let h = human_owned(&r).expect("human owned");
    assert_eq!((h.id, h.owner), ("case_type_taxonomy", "supervision"));
    assert!(map_finding(&r).is_none() && candidate_plan(&r, Caps::default()).is_empty());
    assert!(h.note_es.contains("persona") && h.note_pt.contains("pessoa"));
    let f = pcell("P_SUGG_FAILED", json!({"channel": "app_chat", "language": "es"}));
    assert_eq!(human_owned(&f).unwrap().id, "copilot_failures_platform");
}

#[test]
fn assistant_escalation_on_charge_case_types_points_at_the_dispute_clarify_template_other_types_stay_descriptive() {
    let charge = pcell("P_ASSIST_ESCALATION", json!({"case_type": "undue_charge", "channel": "web_chat"}));
    let row = map_finding(&charge).expect("row");
    assert_eq!(row.id, "assistant_escalation_dispute");
    assert_eq!((row.targets[0].target_ref.as_str(), row.targets[0].agent), ("template:t/aclarar_cargo", "disputas"));
    let card = pcell("P_ASSIST_ESCALATION", json!({"case_type": "virtual_card", "channel": "web_chat"}));
    assert!(map_finding(&card).is_none() && human_owned(&card).is_none(), "no existing agent covers virtual cards: descriptive, no proposal");
}

#[test]
fn bank_and_e0_rows_are_unchanged_by_the_platform_rows() {
    assert_eq!(map_finding(&cell("M1", "Queja", "Phone")).unwrap().id, "complaint_unresolved");
    let e1 = finding_of(signal("E1", json!({"case_type": "dispute"}), stage(560, 1000, 0.17), stage(2800, 5000, 0.17)), Source::E0Treated);
    assert_eq!(map_finding(&e1).unwrap().id, "copilot_repeated_lookup");
}

#[test]
fn platform_aggregates_are_a_derived_source_with_their_own_label_synthetic_stays_synthetic() {
    assert_eq!(Source::parse("platform_treated"), Some(Source::PlatformTreated));
    assert_eq!(Source::parse("platform"), Some(Source::PlatformTreated));
    assert!(Source::PlatformTreated.derived() && !Source::Synthetic.derived());
    assert_eq!(Source::PlatformTreated.as_str(), "platform_treated");
}
