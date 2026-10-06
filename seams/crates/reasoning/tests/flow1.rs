//! FLOW1: the MODEL picks a position and a preset from menus derived from the real flow graph; the compiler and an independent review (structured
//! facts only) decide. Scripted ports; a protected edge, an unknown position, an unprovable ask or a refuting review never become a proposal.
mod common;
use common::*;
use engine::models::ModelRequest;
use reasoning::finding::Source;
use reasoning::mapping::{Caps, Mapped, Table, map_finding};
use reasoning::patch::compile;
use reasoning::pipeline::{candidate_plan, reason_candidate};
use reasoning::roles::{Alt, Opportunity};
use reasoning::testkit::FnPort;
use serde_json::{Value, json};

const VALIDATOR: &str = "flow_edit:consulta-pqr/add_validator";
const ASK: &str = "flow_edit:disputa-cargo/insert_ask";
const NOTICE: &str = "flow_edit:consulta-pqr/insert_notice";

fn pcell(metric: &str, dims: Value) -> reasoning::Finding {
    finding_of(signal(metric, dims, stage(70, 100, 0.35), stage(56, 80, 0.35)), Source::PlatformTreated)
}
fn dispute_finding() -> reasoning::Finding {
    pcell("P_ASSIST_ESCALATION", json!({"case_type": "undue_charge", "channel": "web_chat"}))
}
fn opp(target: &str, mech: &str) -> Opportunity {
    Opportunity { id: "h_1".into(), target_ref: target.into(), mechanism_class: mech.into(), hypothesis: "h".into(), claimed_rate: 0.9, falsifiers: vec!["f".into()], alternatives: vec![Alt { kind: "do_nothing".into(), why_not: "x".into() }] }
}
fn proposal(position: &str, preset: &str) -> Value {
    json!({"kind": "flow_edit", "position": position, "preset": preset, "rationale": "Ask before the search.", "alternatives": alts(), "uncertainty": "Association only."})
}
fn compile_on(f: &reasoning::Finding, target: &str, mech: &str, p: Value) -> Result<reasoning::patch::Compiled, reasoning::patch::Denied> {
    let row = map_finding(f).expect("maps");
    compile(&cat(), f, &row.only(target).expect("candidate of the row"), &opp(target, mech), &p)
}

#[test]
fn the_flow_edit_candidates_sit_in_the_rows_without_moving_an_existing_rank() {
    let m4 = map_finding(&pqr_finding()).unwrap();
    let refs: Vec<&str> = m4.targets.iter().map(|t| t.target_ref.as_str()).collect();
    assert_eq!(refs, vec!["template:t/estado_pqr", VALIDATOR, NOTICE, "flow_edit:consulta-pqr/insert_ack"]);
    assert!(m4.targets[1].announceable_now && !m4.targets[2].announceable_now && !m4.targets[3].announceable_now, "only the validator can be proven natively");
    // the dispute assistant row: the template has no generator (not announceable), so the ask jumps the queue
    let d = map_finding(&dispute_finding()).unwrap();
    assert_eq!(d.targets[0].target_ref, "template:t/aclarar_cargo");
    assert_eq!(candidate_plan(&dispute_finding(), Caps::default()), vec![ASK, "template:t/aclarar_cargo"]);
    // a complaint cell keeps its two patches first
    let q = finding_of(signal("M1", json!({"reason_category": "Queja", "channel": "Phone"}), stage(560, 1000, 0.17), stage(2800, 5000, 0.17)), Source::BankTreated);
    assert_eq!(candidate_plan(&q, Caps::default()), vec!["template:t/estado_pqr", "prompt:p/resumen_radicado"]);
    for t in map_finding(&q).unwrap().targets.iter().filter(|t| t.kind == "flow_edit") {
        assert!(t.target_ref.starts_with("flow_edit:consulta-pqr/"));
    }
}

#[test]
fn the_loader_refuses_a_flow_edit_row_that_is_not_what_the_compiler_can_do() {
    let broken = |edit: &dyn Fn(&mut Value)| {
        let mut j: Value = serde_json::from_str(include_str!("../fixtures/mapping_table.json")).unwrap();
        let row = j["rows"].as_array_mut().unwrap().iter_mut().find(|r| r["id"] == "pqr_status_message").unwrap();
        edit(&mut row["candidates"][1]);
        Table::parse(&j).expect_err("must be refused")
    };
    assert!(broken(&|c| c["target_ref"] = json!("flow_edit:consulta-pqr/delete_node")).contains("does not match kind"));
    assert!(broken(&|c| c["params"]["op"] = json!("insert_ask")).contains("repeat the target ref"));
    assert!(broken(&|c| c["proof_support"] = json!("suite:flow_ask")).contains("proof_support"));
    assert!(broken(&|c| c["params"]["presets"] = json!([])).contains("presets"));
    assert!(broken(&|c| c["params"]["presets"][0]["validator"] = json!({"kind": "decide", "value": "x"})).contains("validator_invalid"));
    assert!(broken(&|c| c["params"]["presets"][0]["reject"] = json!({"es": [], "pt": []})).contains("accept and reject"));
    assert!(broken(&|c| c["mechanisms"] = json!(["invented"])).contains("mechanisms"));
}

#[test]
fn compile_builds_flow_agent_and_template_changes_and_carries_the_suite_params() {
    let f = dispute_finding();
    let c = compile_on(&f, ASK, "flow_missing_context", proposal("pedir_cargo.ok", "ask_fecha")).expect("compiles");
    assert_eq!((c.kind.as_str(), c.agent_id.as_str()), ("flow_edit", "disputas"));
    let kinds: Vec<&str> = c.changes.iter().map(|x| x["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["flow", "agent", "template"]);
    assert_eq!(c.changes[0]["content"]["version"], "1.1.0");
    assert_eq!(c.cascade, vec!["agent:disputas@1.0.1"]);
    let p = &c.expected_effect["art2"]["suite_params"];
    assert_eq!((p["op"].as_str(), p["discriminator"].as_str(), p["flow"].as_str()), (Some("insert_ask"), Some("tool_not_called"), Some("disputa-cargo")));
    assert_eq!(p["chain"][0]["slot"], "descripcion_cargo");
    assert_eq!(c.expected_effect["art2"]["facts"]["pass_through"], true);
    assert!(c.human_items.iter().any(|h| h.contains("byte-identical")));
    // validator: the preset travels with its accept and reject examples for the suite self-test
    let q = pqr_finding();
    let v = compile_on(&q, VALIDATOR, "flow_input_validation", proposal("pedir_radicado", "radicado_token")).expect("compiles");
    assert_eq!(v.changes.len(), 2);
    assert!(v.expected_effect["art2"]["suite_params"]["preset"]["reject"]["es"].as_array().unwrap().len() >= 3);
    assert!(v.human_items.iter().any(|h| h.contains("confirms the format")));
    let n = compile_on(&q, NOTICE, "flow_silent_failure", proposal("consultar.error", "notice_handoff")).expect("compiles");
    assert_eq!(n.expected_effect["art2"]["suite_params"]["from_tool"], "obtener_pqr");
}

#[test]
fn the_compiler_refuses_protected_unknown_unprovable_and_mismatched_proposals() {
    let q = pqr_finding();
    let d = |f: &reasoning::Finding, t: &str, m: &str, p: Value| compile_on(f, t, m, p).expect_err("must be denied").code;
    let v = |pos: &str, preset: &str| d(&q, VALIDATOR, "flow_input_validation", proposal(pos, preset));
    assert_eq!(v("consultar", "radicado_token"), "node_protected");
    assert_eq!(v("nope", "radicado_token"), "position_unknown");
    assert_eq!(v("pedir_radicado", "invented"), "preset_unknown");
    let n = |pos: &str| d(&q, NOTICE, "flow_silent_failure", proposal(pos, "notice_handoff"));
    assert_eq!(n("consultar.ok"), "position_not_allowed");
    assert_eq!(n("esc_tool.x"), "position_unknown");
    assert_eq!(n("pedir_radicado.zzz"), "position_unknown");
    let dp = dispute_finding();
    let a = |pos: &str| d(&dp, ASK, "flow_missing_context", proposal(pos, "ask_fecha"));
    for frozen in ["umbral.true", "coincide.unica", "confirmar.true", "radicar.ok", "verificar.verified"] {
        assert_eq!(a(frozen), "edge_frozen", "{frozen}");
    }
    assert_eq!(a("a_usd.ok"), "no_native_evidence", "an ask the suite could not prove is not proposed");
    assert_eq!(a("aclarar.next"), "position_not_allowed");
    // the kind and the op are the target's, not the model's
    assert_eq!(d(&dp, ASK, "flow_missing_context", json!({"kind": "link_tool", "edge_id": "consultar.ok", "rationale": "x", "alternatives": alts(), "uncertainty": "y"})), "kind_mismatch");
    let mut wrong_op = proposal("pedir_cargo.ok", "ask_fecha");
    wrong_op["op"] = json!("insert_ack");
    assert_eq!(d(&dp, ASK, "flow_missing_context", wrong_op), "op_mismatch");
}

fn builder(position: &'static str, preset: &'static str, expect_menu: &'static str) -> impl Fn(&ModelRequest) -> Result<Value, engine::models::ModelError> + 'static {
    move |req| {
        let tools = req.payload["tools"].as_array().cloned().unwrap_or_default();
        let desc = |name: &str| tools.iter().find(|t| t["tool"].as_str().is_some_and(|x| x.starts_with(name))).and_then(|t| t["description"].as_str()).unwrap_or("").to_string();
        let positions = desc("pulso/flow_positions");
        assert!(positions.contains(expect_menu), "the Builder sees the position menu of the real flow: {positions}");
        assert!(!positions.contains("umbral.true") && !positions.contains("radicar"), "a protected edge is not on the menu: {positions}");
        assert!(positions.lines().all(|l| l.ends_with("native_evidence true")), "ask and validator are offered only where the suite can prove them: {positions}");
        assert!(desc("pulso/flow_presets").contains(preset));
        Ok(json!({"proposal": {"kind": "flow_edit", "position": position, "preset": preset, "rationale": "Re-ask when the text carries no id.", "alternatives": alts(), "uncertainty": "The id format is a convention."}}))
    }
}

/// Verifier port: the claim verification and the flow review (goal "review...") are told apart by the goal.
fn verifier(review: &'static str) -> impl Fn(&ModelRequest) -> Result<Value, engine::models::ModelError> + 'static {
    move |req| {
        if req.payload["goal"].as_str().unwrap_or("").starts_with("review") {
            let f = &req.payload["inputs"]["facts"];
            assert_eq!((f["protected_unchanged"].as_bool(), f["pass_through"].as_bool()), (Some(true), Some(true)), "the reviewer reads recomputed structured facts: {f}");
            assert!(f.get("rationale").is_none() && req.payload["inputs"].get("proposal").is_none(), "no model text reaches the reviewer");
            let r = if review == "supported" { "pass" } else { "fail" };
            return Ok(json!({"verdict": review, "checks": [{"id": "protected_untouched", "result": "pass"}, {"id": "pass_through", "result": "pass"}, {"id": "exits_preserved", "result": r}], "rationale": "Judged from the structured facts."}));
        }
        verifier_ok("supported")(req)
    }
}

fn run(f: &reasoning::Finding, target: &str, mech: &str, b: FnPort, review: &'static str) -> reasoning::pipeline::Reasoned {
    let p = ports(FnPort::scripted("scout", scout_ok(f, target, mech)), FnPort::scripted("verifier", verifier(review)), b);
    reason_candidate(&cat(), f, &p, &opts(), Some(target))
}

#[test]
fn the_model_picks_a_position_and_a_preset_and_an_independent_review_supports_the_edit() {
    let r = run(&pqr_finding(), VALIDATOR, "flow_input_validation", FnPort::scripted("builder", builder("pedir_radicado", "radicado_token", "pedir_radicado |")), "supported");
    assert_eq!((r.status.as_str(), r.reason.as_str()), ("proposed", "compiled"), "{} {}", r.reason, r.detail);
    let c = r.compiled_raw.as_ref().unwrap();
    assert_eq!(c.kind, "flow_edit");
    assert_eq!(r.verification.as_ref().unwrap()["flow_review"]["supported"], true);
    assert_eq!(r.metering["calls"], 4, "scout, claim verifier, builder, flow review");
    let dossier = r.dossier.as_ref().unwrap();
    assert!(dossier.get("error").is_none(), "{dossier}");
    let es = dossier.to_string();
    assert!(es.contains("Edición aditiva del flujo") && es.contains("Edição aditiva do fluxo"), "the dossier explains the edit in es and pt");
    assert_eq!(dossier["announce"], false, "no regression verdict yet");
}

#[test]
fn an_ask_on_the_dispute_flow_is_proposed_from_the_evidence_positions_only() {
    let r = run(&dispute_finding(), ASK, "flow_missing_context", FnPort::scripted("builder", builder("pedir_cargo.ok", "ask_fecha", "pedir_cargo.ok |")), "supported");
    assert_eq!((r.status.as_str(), r.reason.as_str()), ("proposed", "compiled"), "{} {}", r.reason, r.detail);
}

#[test]
fn a_refuting_review_a_protected_position_and_a_wrong_kind_never_become_a_proposal() {
    let r = run(&pqr_finding(), VALIDATOR, "flow_input_validation", FnPort::scripted("builder", builder("pedir_radicado", "radicado_token", "pedir_radicado |")), "refuted");
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.stage.as_str()), ("blocked", "flow_review_refuted", "flow_review"));
    for (pos, code) in [("consultar", "compile_denied:node_protected"), ("umbral", "compile_denied:position_unknown"), ("", "compile_denied:position_unknown")] {
        let b = FnPort::scripted("builder", move |_| Ok(json!({"proposal": {"kind": "flow_edit", "position": pos, "preset": "radicado_token", "rationale": "x", "alternatives": alts(), "uncertainty": "y"}})));
        let r = run(&pqr_finding(), VALIDATOR, "flow_input_validation", b, "supported");
        assert_eq!((r.status.as_str(), r.reason.as_str()), ("blocked", code), "{pos}: {}", r.detail);
        assert!(r.compiled_raw.is_none());
    }
}

#[test]
fn the_bundled_table_still_maps_a_flow_row_for_every_row_it_extended() {
    let Mapped::Row(row) = Table::bundled().classify(&dispute_finding()) else { panic!("row") };
    assert_eq!(row.targets.len(), 3);
    assert!(row.caveats.contains(&"mapping_is_a_hypothesis_of_where_to_intervene"));
}
