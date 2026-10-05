//! LIVE W13 proofs against YOUR OWN local agent-core stack (see docs/dev/W13_PROMPT_AND_AGENT_ANNOUNCE.md): (i) a prompt patch proven
//! natively (agent-core with PR 50) AND by the harness probe and announced, (ii) a text-identical prompt not announced, (iii) a NEW
//! agent (clone-closure of the donor) proven on itself and announced, or its honest blocker. Opt-in: every test is `#[ignore]`d.
//!
//! ```text
//! GATEWAY_TOKEN_AGENT_CORE=<from llm-gateway.env, set by the launcher, never printed> PULSO_PROBE_GATEWAY=http://127.0.0.1:8213 \
//! PULSO_DEV_STACK_DIR=<worktree>/.dev-stack PULSO_STACK_ADDR=127.0.0.1:8203 \
//!   cargo test -j 1 -p registry-writer --test live_w13 -- --ignored --nocapture --test-threads 1
//! ```
//!
//! Only manual-origin evaluation drafts are created for the proofs; nothing is approved, published or promoted. Tokens come from
//! tokens.json (and the gateway consumer token from the process environment) and are never printed.
use core_client::authorizer::Jws;
use reasoning::catalog::Catalog;
use reasoning::dossier::{Labels, Runtime};
use reasoning::finding::{Finding, Source};
use reasoning::mapping::map_finding;
use reasoning::patch::{Compiled, compile};
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

fn token_and_addr() -> (Jws, &'static str, String) {
    let dir = std::env::var("PULSO_DEV_STACK_DIR").expect("set PULSO_DEV_STACK_DIR");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(PathBuf::from(dir).join("tokens.json")).expect("tokens.json")).unwrap();
    let addr = std::env::var("PULSO_STACK_ADDR").unwrap_or_else(|_| "127.0.0.1:8001".into());
    let t = HttpTransport::new(&addr, Duration::from_secs(30));
    let probe = registry_writer::MemoryStore::new();
    let b = Jws::new(v["builder"].as_str().expect("builder token").to_string());
    let w = Writer::new(Config::new(Via::RegistryApi, Environment::LocalStack, b.clone()), &t, &probe);
    match w.fetch_artifact("template:t/estado_pqr") {
        Ok(_) => (b, "engine builder principal", addr),
        Err(_) => (Jws::new(v["admin"].as_str().expect("admin token").to_string()), "local staff admin credential (stand-in)", addr),
    }
}

fn m4_finding() -> Finding {
    let sig: Value = serde_json::from_str(&std::fs::read_to_string(root().join("scripts/regression/fixtures/finding_m4_pqr_status.json")).unwrap()).unwrap();
    let mut s = sig.clone();
    s["status"] = json!("corroborated");
    let (mut f, _) = Finding::from_report(&json!({"signals": [s]}), Source::Synthetic).unwrap();
    f.remove(0)
}

fn tecnico_finding() -> Finding {
    let report: Value = serde_json::from_str(&steps::cells::run(&reasoning::testkit::synthetic_cells_ndjson()).unwrap()).unwrap();
    let (f, _) = Finding::from_report(&report, Source::Synthetic).unwrap();
    f.into_iter().find(|x| x.dims.get("reason_category").map(String::as_str) == Some("Tecnico")).expect("the planted cell is corroborated")
}

fn candidate(name: &str) -> Vec<Value> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(root().join(format!("scripts/regression/fixtures/candidates/{name}.json"))).unwrap()).unwrap();
    v["changes"].as_array().unwrap().clone()
}

fn scripts() -> PythonScripts {
    let py = std::env::var("PULSO_REGRESSION_PYTHON").unwrap_or_else(|_| "python".into());
    PythonScripts { python: py.split_whitespace().map(str::to_string).collect(), script_dir: root().join("scripts/regression"), work: std::env::temp_dir().join("pulso-w13-live"), env: vec![] }
}

fn report(proof: &registry_writer::proof::Proof, t0: std::time::Instant) {
    println!("proof in {}s: outcome={} verdict={} announce={}", t0.elapsed().as_secs(), proof.outcome, proof.verdict, proof.announce);
    println!("reason: {}", proof.story["reason"]);
    println!("native_binding: {}", proof.story["native_binding"]);
    println!("coverage: {}", proof.story["coverage"]);
    println!("story ES: {}", proof.story["story_text"]["es"]);
    println!("base verdict={} native={} failed finding cases={} guards failed={}", proof.story["base"]["verdict"], proof.story["base"]["native_verdict"], proof.story["base"]["failed_cases"].as_array().map_or(0, Vec::len), proof.story["base"]["guards_failed"].as_array().map_or(0, Vec::len));
    for a in proof.story["attempts"].as_array().into_iter().flatten() {
        println!("attempt {}: native={} final={} failed={} guards_failed={} infra_retries={} note={}", a["attempt"], a["native_verdict"], a["verdict"], a["failed_cases"].as_array().map_or(0, Vec::len), a["guards_failed"].as_array().map_or(0, Vec::len), a["infra_retries"].as_array().map_or(0, Vec::len), a["verdict_note"]);
        if let Some(pc) = a["per_case"].as_object() {
            if let Some((id, c)) = pc.iter().find(|(_, c)| c["probe"]["samples"].is_array()) {
                println!("  sample of {id}: {}", c["probe"]["samples"][0]);
            }
        }
    }
    if let Some(pc) = proof.story["base"]["per_case"].as_object() {
        if let Some((id, c)) = pc.iter().find(|(_, c)| c["probe"]["samples"].is_array()) {
            println!("base sample of {id}: {}", c["probe"]["samples"][0]);
        }
    }
    let gi = proof.story["gate_items"].as_array().cloned().unwrap_or_default();
    println!("GateItems (final run): {} total, {} passed", gi.len(), gi.iter().filter(|g| g["passed"] == true).count());
    println!("base problem: {} / {}", proof.story["base"]["problem"], proof.story["base"]["detail"]);
    println!("evaluation drafts (manual origin): {:?}", proof.eval_proposals);
    println!("dossier announce={} reason={}", proof.dossier["announce"], proof.dossier["announce_reason"]);
    for s in ["result", "coverage", "unchanged"] {
        println!("dossier ES {s}: {}", proof.dossier["es"]["sections"][s]);
    }
}

struct Live {
    w_token: Jws,
    cred: &'static str,
    addr: String,
}

fn live() -> Live {
    let (w_token, cred, addr) = token_and_addr();
    println!("credential: {cred}; stack {addr}");
    Live { w_token, cred, addr }
}

fn prompt_case(name: &str, announce_expected: bool) {
    let l = live();
    let long = HttpTransport::new(&l.addr, Duration::from_secs(900));
    let store = FileStore::new(std::env::temp_dir().join(format!("pulso-w13-receipts-{name}-{}.json", std::process::id())));
    let mut cfg = Config::new(Via::RegistryApi, Environment::LocalStack, l.w_token.clone());
    cfg.credential = if l.cred.starts_with("engine") { "engine builder principal" } else { "operator-declared stand-in credential (not the engine builder principal)" };
    let w = Writer::new(cfg, &long, &store);
    let base = w.fetch_artifact("prompt:p/resumen_radicado").expect("live p/resumen_radicado (registry-e2e fixture imported?)");
    let compiled = Compiled {
        kind: "patch".into(),
        target_ref: "prompt:p/resumen_radicado".into(),
        agent_id: "disputas".into(),
        changes: candidate(name),
        diff: vec![],
        base_digest: base.digest(),
        cascade: vec![],
        edit_chars: 90,
        edit_budget: 300,
        human_items: vec![],
        expected_effect: Value::Null,
        rationale: "the closing message names nobody who follows the case up; add the follow-up sentence (scripted compiled proposal, no LLM Builder)".into(),
        uncertainty: String::new(),
    };
    let f = m4_finding();
    let inp = ProofInput { finding: &f, compiled: &compiled, attempts: vec![], base_artifact: Some(&base), labels: Labels { runtime: Runtime::Real, ..Default::default() }, doubles: json!([]), rubric: Value::Null };
    let proofs = FileProofStore::new(std::env::temp_dir().join(format!("pulso-w13-proofs-{name}-{}.json", std::process::id())));
    let t0 = std::time::Instant::now();
    let proof = prove(&w, &scripts(), &proofs, &EvalOptions::default(), &inp);
    report(&proof, t0);
    assert_eq!(proof.announce, announce_expected, "{}", proof.story["reason"]);
    if announce_expected {
        assert_eq!((proof.verdict.as_str(), proof.story["native_binding"]["state"].as_str()), ("regression_suite_proven", Some("candidate_bound")));
        let o = w.deliver(&announce_submission(&f, &compiled, &proof));
        println!("announced delivery: {}", o.to_json());
        assert!(o.delivered(), "{}", o.to_json());
        assert_eq!(o.changes, 2, "patch + eval_suite");
    } else {
        assert!(proof.outcome.starts_with("not_announced:"));
    }
}

#[test]
#[ignore = "needs the local stack with agent-core PR 50 and the local gateway"]
fn live_prompt_patch_is_proven_with_pr50_and_announced() {
    prompt_case("resumen_radicado_attempt2", true);
}

#[test]
#[ignore = "needs the local stack with agent-core PR 50 and the local gateway"]
fn live_text_identical_prompt_is_not_announced() {
    prompt_case("resumen_radicado_noop", false);
}

fn tecnico_compiled(f: &Finding) -> Compiled {
    let row = map_finding(f).unwrap();
    let o = Opportunity {
        id: "h_1".into(),
        target_ref: "new_agent:consultas".into(),
        mechanism_class: "uncovered_topic".into(),
        hypothesis: "h".into(),
        claimed_rate: 0.9,
        falsifiers: vec!["f".into()],
        alternatives: vec![Alt { kind: "do_nothing".into(), why_not: "x".into() }, Alt { kind: "human_owned".into(), why_not: "y".into() }],
    };
    let proposal = json!({"kind": "new_agent", "target_ref": "new_agent:consultas", "agent_id": "soporte-tecnico", "rationale": "A narrow intake for the uncovered technical topic (scripted compiled proposal, no LLM Builder).", "expected_direction": "decrease",
        "routing": {"summary_es": "Recibe problemas t\u{e9}cnicos de la aplicaci\u{f3}n y los pasa a una persona.", "summary_pt": "Recebe problemas t\u{e9}cnicos do aplicativo e os encaminha a uma pessoa.",
                    "examples_es": ["la app se cierra sola", "no puedo entrar a la aplicaci\u{f3}n"], "examples_pt": ["o aplicativo fecha sozinho", "n\u{e3}o consigo entrar no aplicativo"]},
        "intake": {"ask_es": "Cu\u{e9}ntame qu\u{e9} problema tienes con la aplicaci\u{f3}n.", "ask_pt": "Conte qual problema voc\u{ea} tem com o aplicativo.",
                   "notice_es": "Gracias, una persona del equipo te contactar\u{e1}.", "notice_pt": "Obrigado, uma pessoa da equipe vai falar com voc\u{ea}."},
        "alternatives": [{"kind": "do_nothing", "why_not": "x"}, {"kind": "human_owned", "why_not": "y"}], "uncertainty": "Where is known, why is not."});
    compile(&Catalog::bundled(), f, &row, &o, &proposal).expect("the scripted new-agent proposal compiles")
}

#[test]
#[ignore = "needs the local stack (registry-e2e fixture) and agent-core with the evaluation harness"]
fn live_new_agent_is_proven_on_itself_and_announced() {
    let l = live();
    let long = HttpTransport::new(&l.addr, Duration::from_secs(900));
    let store = FileStore::new(std::env::temp_dir().join(format!("pulso-w13-receipts-agent-{}.json", std::process::id())));
    let mut cfg = Config::new(Via::RegistryApi, Environment::LocalStack, l.w_token.clone());
    cfg.credential = if l.cred.starts_with("engine") { "engine builder principal" } else { "operator-declared stand-in credential (not the engine builder principal)" };
    let w = Writer::new(cfg, &long, &store);
    let f = tecnico_finding();
    let compiled = tecnico_compiled(&f);
    let inp = ProofInput { finding: &f, compiled: &compiled, attempts: vec![], base_artifact: None, labels: Labels { runtime: Runtime::Real, ..Default::default() }, doubles: json!([]), rubric: Value::Null };
    let proofs = FileProofStore::new(std::env::temp_dir().join(format!("pulso-w13-proofs-agent-{}.json", std::process::id())));
    let t0 = std::time::Instant::now();
    let proof = prove(&w, &scripts(), &proofs, &EvalOptions::default(), &inp);
    report(&proof, t0);
    if let Some(pc) = proof.story["attempts"][0]["per_case"].as_object() {
        for (id, c) in pc.iter().filter(|(_, c)| c["passed"] == false) {
            println!("failed case {id}: {}", c["reason"]);
        }
    }
    println!("candidate problem: {}", proof.story["attempts"][0]["problem"]);
    assert!(proof.announce, "{} | {}", proof.outcome, proof.story["reason"]);
    let o = w.deliver(&announce_submission(&f, &compiled, &proof));
    println!("announced delivery: {}", o.to_json());
    assert!(o.delivered(), "{}", o.to_json());
    let t = HttpTransport::new(&l.addr, Duration::from_secs(30));
    let pid = o.proposal_id.unwrap();
    let r = registry_writer::Transport::send(&t, &registry_writer::Request { method: "GET", path: format!("/v1/registry/proposals/{pid}"), bearer: &l.w_token, idempotency_key: None, body: None }).unwrap();
    let chs = r.body["changes"].as_array().cloned().unwrap_or_default();
    println!("read-back: agent={} origin={} state={} changes={} kinds={:?}", r.body["proposal"]["agent_id"], r.body["proposal"]["origin"], r.body["proposal"]["state"], chs.len(), chs.iter().map(|c| c["kind"].clone()).collect::<Vec<_>>());
    assert_eq!(r.body["proposal"]["origin"], "auto_detect");
    assert!(!chs.iter().any(|c| c["kind"] == "release_settings"));
}
