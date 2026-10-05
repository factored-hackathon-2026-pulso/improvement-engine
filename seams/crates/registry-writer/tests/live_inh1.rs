//! LIVE INH1 proof against YOUR OWN local agent-core stack (agent-core = origin/main + PR 50 + PR 51 on a local scratch branch): the
//! value loop for an uncovered-topic finding (planted cell Tecnico/Phone) builds a NEW agent `soporte-tecnico`, proves it on itself and
//! ANNOUNCES it with the engine `builder` principal ONLY (no admin stand-in): the evaluation drafts and the announced proposal carry
//! `release_settings: {inherit_from: <donor release>}`, the engine never writes interrupts, never approves/publishes/promotes.
//!
//! ```text
//! GATEWAY_TOKEN_AGENT_CORE=<set by the launcher, never printed> PULSO_DEV_STACK_DIR=<worktree>/.dev-stack PULSO_STACK_ADDR=127.0.0.1:8251 //!   PULSO_REGRESSION_PYTHON="uv run --with pyyaml python" //!   cargo test -j 1 -p registry-writer --test live_inh1 -- --ignored --nocapture --test-threads 1
//! ```
use core_client::authorizer::Jws;
use reasoning::catalog::Catalog;
use reasoning::dossier::{Labels, Runtime};
use reasoning::finding::{Finding, Source};
use reasoning::mapping::map_finding;
use reasoning::patch::{Compiled, compile};
use reasoning::roles::{Alt, Opportunity};
use registry_writer::eval::EvalOptions;
use registry_writer::proof::{FileProofStore, ProofInput, PythonScripts, announce_submission, prove};
use registry_writer::{Config, Environment, FileStore, HttpTransport, Reply, Request, Transport, TransportError, Via, Writer};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Records every request the engine sends (method + path shape) so the test can show that approve/publish/promote never travel.
struct Recording<'a> {
    inner: &'a HttpTransport,
    log: RefCell<Vec<String>>,
}

impl Transport for Recording<'_> {
    fn send(&self, req: &Request) -> Result<Reply, TransportError> {
        let shape: Vec<&str> = req.path.split('?').next().unwrap().split('/').map(|s| if s.starts_with("prp_") || s.starts_with("rel-") { "{id}" } else { s }).collect();
        self.log.borrow_mut().push(format!("{} {}", req.method, shape.join("/")));
        self.inner.send(req)
    }
}

fn tecnico_finding() -> Finding {
    let report: Value = serde_json::from_str(&steps::cells::run(&reasoning::testkit::synthetic_cells_ndjson()).unwrap()).unwrap();
    let (f, _) = Finding::from_report(&report, Source::Synthetic).unwrap();
    f.into_iter().find(|x| x.dims.get("reason_category").map(String::as_str) == Some("Tecnico")).expect("the planted cell is corroborated")
}

/// A re-run on the same stack must not replay the frozen drafts of an earlier run (registry Idempotency-Key replay): the optional
/// PULSO_LIVE_NONCE is appended to the changelog of the candidate, which changes the candidate digest and so the evaluation keys.
fn nonce(changes: &mut [Value]) {
    if let Ok(n) = std::env::var("PULSO_LIVE_NONCE") {
        for c in changes.iter_mut().filter(|c| c["docs"]["changelog"].is_string()) {
            let t = format!("{} [run {n}]", c["docs"]["changelog"].as_str().unwrap());
            c["docs"]["changelog"] = json!(t);
        }
    }
}

fn scripts() -> PythonScripts {
    let py = std::env::var("PULSO_REGRESSION_PYTHON").unwrap_or_else(|_| "python".into());
    PythonScripts { python: py.split_whitespace().map(str::to_string).collect(), script_dir: root().join("scripts/regression"), work: std::env::temp_dir().join("pulso-inh1-live"), env: vec![] }
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
    let mut c = compile(&Catalog::bundled(), f, &row, &o, &proposal).expect("the scripted new-agent proposal compiles");
    nonce(&mut c.changes);
    c
}

#[test]
#[ignore = "needs the local INH1 stack (agent-core main + PR 50 + PR 51) and the local gateway"]
fn live_new_agent_is_proven_and_announced_with_the_builder_principal_only_inheriting_the_donor_settings() {
    let dir = std::env::var("PULSO_DEV_STACK_DIR").expect("set PULSO_DEV_STACK_DIR");
    let tokens: Value = serde_json::from_str(&std::fs::read_to_string(PathBuf::from(dir).join("tokens.json")).expect("tokens.json")).unwrap();
    let addr = std::env::var("PULSO_STACK_ADDR").expect("set PULSO_STACK_ADDR");
    let builder = Jws::new(tokens["builder"].as_str().expect("builder token").to_string());
    let long = HttpTransport::new(&addr, Duration::from_secs(900));
    let rec = Recording { inner: &long, log: RefCell::new(vec![]) };
    let store = FileStore::new(std::env::temp_dir().join(format!("pulso-inh1-receipts-{}.json", std::process::id())));
    // NO admin stand-in anywhere: one Writer, the engine builder principal, no fallback configured.
    let cfg = Config::new(Via::RegistryApi, Environment::LocalStack, builder.clone());
    assert!(!cfg.admin_settings_fallback);
    let w = Writer::new(cfg, &rec, &store);
    let f = tecnico_finding();
    let compiled = tecnico_compiled(&f);
    let inp = ProofInput { finding: &f, compiled: &compiled, attempts: vec![], base_artifact: None, labels: Labels { runtime: Runtime::Real, ..Default::default() }, doubles: json!([]), rubric: Value::Null };
    let proofs = FileProofStore::new(std::env::temp_dir().join(format!("pulso-inh1-proofs-{}.json", std::process::id())));
    let t0 = std::time::Instant::now();
    let proof = prove(&w, &scripts(), &proofs, &EvalOptions::default(), &inp);
    report(&proof, t0);
    println!("candidate problem: {}", proof.story["attempts"][0]["problem"]);
    if let Some(pc) = proof.story["attempts"][0]["per_case"].as_object() {
        for (id, c) in pc.iter().filter(|(_, c)| c["passed"] == false) {
            println!("failed case {id}: {}", c["reason"]);
        }
    }
    assert!(proof.announce, "BLOCKER {} | {}", proof.outcome, proof.story["reason"]);
    let sub = announce_submission(&f, &compiled, &proof);
    let o = w.deliver(&sub);
    println!("announced delivery: {}", o.to_json());
    assert!(o.delivered(), "{}", o.to_json());
    let pid = o.proposal_id.clone().unwrap();
    let get = |path: String| long.send(&Request { method: "GET", path, bearer: &builder, idempotency_key: None, body: None }).unwrap();
    let r = get(format!("/v1/registry/proposals/{pid}"));
    let chs = r.body["changes"].as_array().cloned().unwrap_or_default();
    let rs = chs.iter().find(|c| c["kind"] == "release_settings").cloned().unwrap_or(Value::Null);
    println!("READ-BACK proposal={pid} agent={} origin={} state={} created_by={} changes={} kinds={:?}", r.body["proposal"]["agent_id"], r.body["proposal"]["origin"], r.body["proposal"]["state"], r.body["proposal"]["created_by"], chs.len(), chs.iter().map(|c| c["kind"].clone()).collect::<Vec<_>>());
    println!("READ-BACK release_settings change content: {}", rs["content"]);
    let donor = get("/v1/registry/aliases/consultas/prod".into());
    let donor_id = donor.body["release_id"].as_str().unwrap().to_string();
    let rel = get(format!("/v1/registry/releases/{donor_id}"));
    println!("DONOR release {donor_id} status={} interrupts={} language_detection={} injection_ruleset={} max_input_chars={}", rel.body["status"], rel.body["interrupts"], rel.body["language_detection"], rel.body["injection_ruleset"], rel.body["max_input_chars"]);
    assert_eq!((r.body["proposal"]["origin"].as_str(), r.body["proposal"]["state"].as_str()), (Some("auto_detect"), Some("draft")));
    assert_eq!(rs["content"], json!({"inherit_from": donor_id}));
    assert!(chs.iter().all(|c| c["content"].get("interrupts").is_none() || c["kind"] == "agent"), "the engine wrote no interrupts");
    // routes used by the engine in the whole run
    let mut shapes: Vec<String> = rec.log.borrow().clone();
    shapes.sort();
    shapes.dedup();
    println!("ROUTES USED ({} requests): {shapes:?}", rec.log.borrow().len());
    assert!(!rec.log.borrow().iter().any(|l| ["approve", "publish", "promote", "reject", "revoke", "reopen"].iter().any(|w| l.contains(w))), "approve/publish/promote must never be called");
    let fresh = get(format!("/v1/registry/proposals/{pid}"));
    assert_eq!(fresh.body["proposal"]["state"], "draft", "nothing moved the announced proposal");
}
