//! ART3: the MODEL drives a tool link (Scout -> Verifier -> Builder picks an edge -> compiler -> independent link review). Scripted ports; the
//! compiler and the reviewer are what refuse a write tool, an invalid edge or an unsupported link, whatever the model proposes.
mod common;
use common::*;
use engine::models::ModelRequest;
use reasoning::finding::Source;
use reasoning::mapping::Table;
use reasoning::pipeline::reason_candidate;
use reasoning::testkit::FnPort;
use serde_json::{Value, json};

fn a5() -> reasoning::Finding {
    finding_of(signal("A5", json!({"agent": "copiloto-asesor", "tool": "leer_pqr_cliente"}), stage(150, 200, 0.4), stage(120, 160, 0.4)), Source::Synthetic)
}

fn builder_edge(edge: &'static str) -> impl Fn(&ModelRequest) -> Result<Value, engine::models::ModelError> + 'static {
    move |req| {
        let menu = req.payload["tools"][0]["description"].as_str().unwrap_or("").to_string();
        assert!(menu.contains("consultar.ok |"), "the Builder sees the edge menu of the real flow: {menu}");
        assert_eq!(req.payload["inputs"]["tool"], "leer_pqr_cliente");
        Ok(json!({"proposal": {"kind": "link_tool", "edge_id": edge, "rationale": "Read the customer cases before the status answer.", "alternatives": alts(), "uncertainty": "Association only."}}))
    }
}

/// Verifier port: the claim verification (goal "verify...") and the link review (goal "review...") are told apart by the goal.
fn verifier(review: &'static str) -> impl Fn(&ModelRequest) -> Result<Value, engine::models::ModelError> + 'static {
    move |req| {
        if req.payload["goal"].as_str().unwrap_or("").starts_with("review") {
            assert_eq!(req.payload["inputs"]["risk_class"], "read");
            let r = if review == "supported" { "pass" } else { "fail" };
            return Ok(json!({"verdict": review, "checks": [{"id": "read_only", "result": "pass"}, {"id": "edge_valid", "result": r}], "rationale": "Judged from the structured facts."}));
        }
        verifier_ok("supported")(req)
    }
}

fn run(edge: &'static str, review: &'static str) -> reasoning::pipeline::Reasoned {
    let f = a5();
    let p = ports(FnPort::scripted("scout", scout_ok(&f, "tool_link:consultas/leer_pqr_cliente", "missing_tool")), FnPort::scripted("verifier", verifier(review)), FnPort::scripted("builder", builder_edge(edge)));
    reason_candidate(&cat(), &f, &p, &opts(), Some("tool_link:consultas/leer_pqr_cliente"))
}

#[test]
fn the_model_picks_an_edge_and_an_independent_review_supports_a_read_only_link() {
    let r = run("consultar.ok", "supported");
    assert_eq!((r.status.as_str(), r.reason.as_str()), ("proposed", "compiled"), "{} {}", r.reason, r.detail);
    let c = r.compiled_raw.as_ref().unwrap();
    assert_eq!(c.kind, "link_tool");
    assert_eq!(r.verification.as_ref().unwrap()["link_review"]["supported"], true);
    assert_eq!(r.metering["calls"], 4, "scout, claim verifier, builder, link review");
}

#[test]
fn an_invalid_edge_is_refused_by_the_compiler_even_if_the_model_proposes_it() {
    for edge in ["umbral.true", "responder.next", "nope.ok", ""] {
        let r = run(edge, "supported");
        assert_eq!((r.status.as_str(), r.reason.as_str()), ("blocked", "compile_denied:edge_unknown"), "{edge}: {}", r.detail);
        assert!(r.compiled_raw.is_none());
    }
}

#[test]
fn a_refuting_review_blocks_the_link() {
    let r = run("consultar.ok", "refuted");
    assert_eq!((r.status.as_str(), r.reason.as_str(), r.stage.as_str()), ("blocked", "link_review_refuted", "link_review"));
}

#[test]
fn a_write_tool_target_is_refused_by_the_compiler() {
    // a (hypothetical, mis-authored) mapping row that targets a WRITE tool: the table loader accepts the shape, the compiler refuses the tool
    let mut j: Value = serde_json::from_str(include_str!("../fixtures/mapping_table.json")).unwrap();
    let rows = j["rows"].as_array_mut().unwrap();
    let row = rows.iter_mut().find(|r| r["id"] == "tool_peers_gap").unwrap();
    row["candidates"][0]["target_ref"] = json!("tool_link:consultas/radicar_pqr");
    row["candidates"][0]["params"] = json!({"tool": "radicar_pqr"});
    let table = Table::parse(&j).expect("shape is valid");
    let f = a5();
    let reasoning::mapping::Mapped::Row(row) = table.classify(&f) else { panic!("maps") };
    let o = reasoning::roles::Opportunity { id: "h_1".into(), target_ref: "tool_link:consultas/radicar_pqr".into(), mechanism_class: "missing_tool".into(), hypothesis: "h".into(), claimed_rate: 0.75, falsifiers: vec!["f".into()], alternatives: vec![] };
    let e = reasoning::patch::compile(&cat(), &f, &row, &o, &json!({"kind": "link_tool", "edge_id": "consultar.ok", "rationale": "x", "alternatives": alts(), "uncertainty": "y"})).unwrap_err();
    assert_eq!(e.code, "write_tool_human_only");
}
