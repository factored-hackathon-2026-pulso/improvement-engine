//! LIVE ART2 proofs against YOUR OWN local agent-core stack (`scripts/battery/demo_core.py up` with PULSO_STACK_PREFIX=pulso-art2 and the
//! ALIGNED registry-e2e import): (i) a read-only tool link on `consultas` proven (fails on base, passes on candidate) and announced as an
//! auto_detect draft; (ii) a tighten-only draft of the human-owned policy `escalamiento-disputa-monto` proven natively with the boundary
//! scenarios and recorded `needs_owner_ack` (not announced). Opt-in (`#[ignore]`).
//!
//! ```text
//! PULSO_DEV_STACK_DIR=<worktree>/.dev-stack/battery-pulso-art2 PULSO_STACK_ADDR=127.0.0.1:8082 \
//!   cargo test -j 1 -p registry-writer --test live_art2 -- --ignored --nocapture --test-threads 1
//! ```
//!
//! The credential is the local staff admin stand-in (declared in every delivery label); tokens are read from tokens.json and never printed.
use core_client::authorizer::Jws;
use reasoning::catalog::Catalog;
use reasoning::dossier::{Labels, Runtime};
use reasoning::finding::{Finding, Source};
use reasoning::roles::{Alt, Opportunity};
use registry_writer::eval::EvalOptions;
use registry_writer::proof::{FileProofStore, ProofInput, PythonScripts, announce_submission, prove};
use registry_writer::{Config, Environment, FileStore, HttpTransport, Via, Writer};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn admin() -> (Jws, String) {
    let dir = std::env::var("PULSO_DEV_STACK_DIR").expect("set PULSO_DEV_STACK_DIR");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(PathBuf::from(dir).join("tokens.json")).expect("tokens.json")).unwrap();
    (Jws::new(v["admin"].as_str().expect("admin token").to_string()), std::env::var("PULSO_STACK_ADDR").unwrap_or_else(|_| "127.0.0.1:8082".into()))
}

fn stage(n: i64, d: i64) -> Value {
    let r = n as f64 / d as f64;
    json!({"numerator": n, "denominator": d, "rate": r, "baseline_rate": 0.4, "diff": r - 0.4, "p": 0.0})
}

fn mk(metric: &str, dims: Value) -> Finding {
    let sig = json!({"metric": metric, "dims": dims, "status": "corroborated", "reason": "replicated", "direction": "up", "claim": "association", "discovery": stage(150, 200), "holdout": stage(120, 160), "r2": {"status": "replicated"}, "p_adj": 0.0});
    Finding::from_report(&json!({"semantics": "claude-standin", "cells_explored": 12, "signals": [sig], "discards": []}), Source::Synthetic).unwrap().0.remove(0)
}

fn scripts() -> PythonScripts {
    let py = std::env::var("PULSO_REGRESSION_PYTHON").unwrap_or_else(|_| "python".into());
    let work = std::env::temp_dir().join("pulso-art2-live");
    std::fs::create_dir_all(&work).unwrap();
    PythonScripts { python: py.split_whitespace().map(str::to_string).collect(), script_dir: root().join("scripts/regression"), work, env: vec![] }
}

fn writer<'a>(t: &'a HttpTransport, store: &'a FileStore, tok: &Jws) -> Writer<'a> {
    let mut cfg = Config::new(Via::RegistryApi, Environment::LocalStack, tok.clone());
    cfg.credential = "operator-declared stand-in credential (not the engine builder principal)";
    Writer::new(cfg, t, store)
}

/// The fixture catalogue with the LIVE entities of the registry for every ART2 entity (flows, agents, policy, ToolDefs).
fn live_catalog(w: &Writer) -> Catalog {
    let mut c = Catalog::bundled();
    let mut same = vec![];
    for (kind, id) in [("flow", "consulta-pqr"), ("agent", "consultas"), ("tool", "leer_pqr_cliente"), ("flow", "disputa-cargo"), ("agent", "disputas"), ("policy", "escalamiento-disputa-monto")] {
        let live = w.fetch_content(kind, id).unwrap_or_else(|| panic!("live {kind} {id} cannot be read"));
        let fixture = match kind {
            "flow" => c.flow(id).cloned(),
            "policy" => c.policy(id).cloned(),
            "tool" => c.tool_def(id).cloned(),
            _ => c.agent(id).cloned(),
        };
        same.push(format!("{kind}:{id}={}", fixture.as_ref() == Some(&live)));
        c.put_entity(kind, id, live);
    }
    println!("live entities equal to the fixture baseline: {}", same.join(" "));
    c.label = "live-registry".into();
    c
}

fn report(p: &registry_writer::proof::Proof, t0: std::time::Instant) {
    println!("proof in {}s: outcome={} verdict={} announce={}", t0.elapsed().as_secs(), p.outcome, p.verdict, p.announce);
    println!("reason: {}", p.story["reason"]);
    println!("coverage: {}", p.story["coverage"]);
    println!("story ES: {}", p.story["story_text"]["es"]);
    println!("base: native={} failed finding cases={} guards failed={} problem={}", p.story["base"]["native_verdict"], p.story["base"]["failed_cases"].as_array().map_or(0, Vec::len), p.story["base"]["guards_failed"].as_array().map_or(0, Vec::len), p.story["base"]["problem"]);
    for a in p.story["attempts"].as_array().into_iter().flatten() {
        println!("candidate: native={} final={} failed={} guards_failed={} infra_retries={} problem={}", a["native_verdict"], a["verdict"], a["failed_cases"].as_array().map_or(0, Vec::len), a["guards_failed"].as_array().map_or(0, Vec::len), a["infra_retries"].as_array().map_or(0, Vec::len), a["problem"]);
        if let Some(pc) = a["per_case"].as_object() {
            for (id, c) in pc.iter().filter(|(_, c)| c["passed"] == false) {
                println!("  failing {id}: {}", c["reason"]);
            }
        }
    }
    if let Some(pc) = p.story["base"]["per_case"].as_object() {
        println!("base per case: {}", pc.iter().map(|(k, v)| format!("{k}={}", v["passed"])).collect::<Vec<_>>().join(" "));
    }
    let gi = p.story["gate_items"].as_array().cloned().unwrap_or_default();
    println!("GateItems (final run): {} total, {} passed", gi.len(), gi.iter().filter(|g| g["passed"] == true).count());
    for g in gi.iter().take(40) {
        println!("  gate {} phase={} passed={} value={}", g["metric"], g["phase"], g["passed"], g["value"]);
    }
    println!("evaluation drafts (manual origin): {:?}", p.eval_proposals);
    println!("dossier announce={} reason={}", p.dossier["announce"], p.dossier["announce_reason"]);
    println!("dossier ES result: {}", p.dossier["es"]["sections"]["result"]);
}

#[test]
#[ignore = "needs the own local stack with the aligned registry-e2e import"]
fn live_read_only_tool_link_is_proven_and_announced_as_auto_detect() {
    let (tok, addr) = admin();
    let long = HttpTransport::new(&addr, Duration::from_secs(900));
    let store = FileStore::new(std::env::temp_dir().join(format!("pulso-art2-receipts-link-{}.json", std::process::id())));
    let w = writer(&long, &store, &tok);
    let cat = live_catalog(&w);
    let f = mk("A5", json!({"agent": "copiloto-asesor", "tool": "leer_pqr_cliente"}));
    let row = reasoning::mapping::map_finding(&f).expect("A5 on leer_pqr_cliente maps");
    let o = Opportunity { id: "h_1".into(), target_ref: "tool_link:consultas/leer_pqr_cliente".into(), mechanism_class: "missing_tool".into(), hypothesis: "h".into(), claimed_rate: 0.75, falsifiers: vec!["f".into()], alternatives: vec![Alt { kind: "do_nothing".into(), why_not: "x".into() }] };
    let menu: Vec<String> = reasoning::art2::edge_menu(cat.flow("consulta-pqr").unwrap()).iter().map(|e| e["edge_id"].as_str().unwrap().to_string()).collect();
    println!("edge menu derived from the live flow: {menu:?}");
    let mut compiled = reasoning::patch::compile(&cat, &f, &row, &o, &json!({"kind": "link_tool", "edge_id": "consultar.ok", "rationale": "Add the read of the customer cases (scripted compiled proposal, no LLM Builder).", "alternatives": [], "uncertainty": "Association only."})).expect("the link compiles on the live entities");
    if let Ok(n) = std::env::var("PULSO_LIVE_NONCE") {
        compiled.changes[0]["docs"]["changelog"] = json!(format!("link to existing tool, read-only [run {n}]"));
    }
    let inp = ProofInput { finding: &f, compiled: &compiled, attempts: vec![], base_artifact: None, labels: Labels { runtime: Runtime::Real, ..Default::default() }, doubles: json!([]), rubric: Value::Null };
    let proofs = FileProofStore::new(std::env::temp_dir().join(format!("pulso-art2-proofs-link-{}.json", std::process::id())));
    let t0 = std::time::Instant::now();
    let p = prove(&w, &scripts(), &proofs, &EvalOptions::default(), &inp);
    report(&p, t0);
    assert!(p.announce, "{} / {}", p.outcome, p.story["reason"]);
    let o = w.deliver(&announce_submission(&f, &compiled, &p));
    println!("announced delivery: {}", o.to_json());
    assert!(o.delivered(), "{}", o.to_json());
    assert_eq!(o.changes, 4, "flow + agent + tool copy + eval_suite");
}

#[test]
#[ignore = "needs the own local stack with the aligned registry-e2e import"]
fn live_tighten_only_policy_is_proven_natively_and_recorded_needs_owner_ack() {
    let (tok, addr) = admin();
    let long = HttpTransport::new(&addr, Duration::from_secs(900));
    let store = FileStore::new(std::env::temp_dir().join(format!("pulso-art2-receipts-pol-{}.json", std::process::id())));
    let w = writer(&long, &store, &tok);
    let cat = live_catalog(&w);
    let f = mk("E2", json!({"reason_code": "policy:escalamiento-disputa-monto"}));
    let h = reasoning::mapping::human_owned(&f).expect("the dispute amount policy is human owned");
    let mut compiled = reasoning::patch::compile_policy_tighten(&cat, &h.policy).expect("the tighten-only draft compiles");
    println!("owner {} threshold {} -> {}", compiled.changes[0]["content"]["owner"], compiled.diff[0]["threshold"]["from"], compiled.diff[0]["threshold"]["to"]);
    if let Ok(n) = std::env::var("PULSO_LIVE_NONCE") {
        compiled.changes[0]["docs"]["changelog"] = json!(format!("threshold moved towards more escalation [run {n}]"));
    }
    let inp = ProofInput { finding: &f, compiled: &compiled, attempts: vec![], base_artifact: None, labels: Labels { runtime: Runtime::Real, ..Default::default() }, doubles: json!([]), rubric: Value::Null };
    let proofs = FileProofStore::new(std::env::temp_dir().join(format!("pulso-art2-proofs-pol-{}.json", std::process::id())));
    let t0 = std::time::Instant::now();
    let p = prove(&w, &scripts(), &proofs, &EvalOptions::default(), &inp);
    report(&p, t0);
    assert!(!p.announce, "a human-owned policy is never announced automatically");
    assert_eq!(p.outcome, "needs_owner_ack", "{}", p.story["reason"]);
}
