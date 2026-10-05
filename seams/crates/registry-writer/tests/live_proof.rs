//! LIVE W11 proof against YOUR OWN local agent-core stack (scripts/dev-stack/stack.py with PULSO_STACK_PREFIX and own ports, the
//! registry-e2e fixture, PULSO_SERVE_E2E=1; see docs/dev/W11_EVALUATE_BEFORE_ANNOUNCE.md). Opt-in: every test is `#[ignore]`d.
//!
//! ```text
//! PULSO_DEV_STACK_DIR=<worktree>/.dev-stack PULSO_STACK_ADDR=127.0.0.1:8161 \
//!   cargo test -j 1 -p registry-writer --test live_proof -- --ignored --nocapture --test-threads 1
//! ```
//!
//! A SCRIPTED compiled proposal for the `t/estado_pqr` template finding (no LLM Builder): the proven candidate must be announced
//! (dossier ES text in the delivered proposal, origin auto_detect) and a non-improving candidate must NOT be announced. Only manual
//! evaluation drafts are created for the proof; nothing is approved, published or promoted. Tokens come from tokens.json and are never
//! printed; the python subprocesses get no credential.
use core_client::authorizer::Jws;
use reasoning::dossier::{Labels, Runtime};
use reasoning::finding::{Finding, Source};
use reasoning::patch::Compiled;
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
    // the engine kid is trusted by the local staff keys (identity.py); otherwise the staff admin stands in (declared)
    let t = HttpTransport::new(&addr, Duration::from_secs(30));
    let probe = registry_writer::MemoryStore::new();
    let b = Jws::new(v["builder"].as_str().expect("builder token").to_string());
    let w = Writer::new(Config::new(Via::RegistryApi, Environment::LocalStack, b.clone()), &t, &probe);
    match w.fetch_artifact("template:t/estado_pqr") {
        Ok(_) => (b, "engine builder principal", addr),
        Err(_) => (Jws::new(v["admin"].as_str().expect("admin token").to_string()), "local staff admin credential (stand-in)", addr),
    }
}

fn finding() -> Finding {
    let sig: Value = serde_json::from_str(&std::fs::read_to_string(root().join("scripts/regression/fixtures/finding_m4_pqr_status.json")).unwrap()).unwrap();
    let mut s = sig.clone();
    s["status"] = json!("corroborated");
    let (mut f, _) = Finding::from_report(&json!({"signals": [s]}), Source::Synthetic).unwrap();
    f.remove(0)
}

fn candidate(name: &str) -> Vec<Value> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(root().join(format!("scripts/regression/fixtures/candidates/{name}.json"))).unwrap()).unwrap();
    v["changes"].as_array().unwrap().clone()
}

fn go(name: &str, announce_expected: bool) {
    let (token, cred, addr) = token_and_addr();
    println!("credential: {cred}; stack {addr}");
    let long = HttpTransport::new(&addr, Duration::from_secs(900));
    let store = FileStore::new(std::env::temp_dir().join(format!("pulso-w11-receipts-{name}-{}.json", std::process::id())));
    let mut cfg = Config::new(Via::RegistryApi, Environment::LocalStack, token);
    cfg.credential = if cred.starts_with("engine") { "engine builder principal" } else { "operator-declared stand-in credential (not the engine builder principal)" };
    let w = Writer::new(cfg, &long, &store);
    let base = w.fetch_artifact("template:t/estado_pqr").expect("live t/estado_pqr (registry-e2e fixture imported?)");
    let compiled = Compiled {
        kind: "patch".into(),
        target_ref: "template:t/estado_pqr".into(),
        agent_id: "consultas".into(),
        changes: candidate(name),
        diff: vec![],
        base_digest: base.digest(),
        cascade: vec![],
        edit_chars: 90,
        edit_budget: 300,
        human_items: vec![],
        expected_effect: Value::Null,
        rationale: "the status sentence names no state; add the status placeholder (scripted compiled proposal, no LLM Builder)".into(),
        uncertainty: String::new(),
    };
    let py = std::env::var("PULSO_REGRESSION_PYTHON").unwrap_or_else(|_| "python".into());
    let scripts = PythonScripts { python: py.split_whitespace().map(str::to_string).collect(), script_dir: root().join("scripts/regression"), work: std::env::temp_dir().join("pulso-w11-live") };
    let f = finding();
    let inp = ProofInput { finding: &f, compiled: &compiled, attempts: vec![], base_artifact: Some(&base), labels: Labels { runtime: Runtime::Real, ..Default::default() }, doubles: json!([]), rubric: Value::Null };
    let proofs = FileProofStore::new(std::env::temp_dir().join(format!("pulso-w11-proofs-{name}-{}.json", std::process::id())));
    let t0 = std::time::Instant::now();
    let proof = prove(&w, &scripts, &proofs, &EvalOptions::default(), &inp);
    println!("proof in {}s: outcome={} verdict={} announce={}", t0.elapsed().as_secs(), proof.outcome, proof.verdict, proof.announce);
    println!("reason: {}", proof.story["reason"]);
    println!("story ES: {}", proof.story["story_text"]["es"]);
    println!("base failed finding cases: {}; guards failed on base: {}", proof.story["base"]["failed_cases"].as_array().map_or(0, Vec::len), proof.story["base"]["guards_failed"].as_array().map_or(0, Vec::len));
    for a in proof.story["attempts"].as_array().into_iter().flatten() {
        println!("attempt {}: native={} final={} failed={} guards_failed={} infra_retries={}", a["attempt"], a["native_verdict"], a["verdict"], a["failed_cases"].as_array().map_or(0, Vec::len), a["guards_failed"].as_array().map_or(0, Vec::len), a["infra_retries"].as_array().map_or(0, Vec::len));
    }
    let gi = proof.story["gate_items"].as_array().cloned().unwrap_or_default();
    println!("GateItems (final run): {} total, {} passed; first: {}", gi.len(), gi.iter().filter(|g| g["passed"] == true).count(), gi.first().map_or(Value::Null, |g| json!({"metric": g["metric"], "phase": g["phase"], "passed": g["passed"]})));
    println!("base problem: {} / {}", proof.story["base"]["problem"], proof.story["base"]["detail"]);
    println!("evaluation drafts (manual origin): {:?}", proof.eval_proposals);
    println!("dossier announce={} reason={} judge_family={} calibration={}", proof.dossier["announce"], proof.dossier["announce_reason"], proof.dossier["honesty"]["judge_family"], proof.dossier["honesty"]["calibration"]);
    println!("dossier ES result: {}", proof.dossier["es"]["sections"]["result"]);
    assert_eq!(proof.announce, announce_expected, "{}", proof.story["reason"]);
    if announce_expected {
        assert_eq!(proof.verdict, "regression_suite_proven");
        let o = w.deliver(&announce_submission(&f, &compiled, &proof));
        println!("announced delivery: {}", o.to_json());
        assert!(o.delivered(), "{}", o.to_json());
        assert_eq!(o.changes, 2, "patch + eval_suite");
        let pid = o.proposal_id.unwrap();
        let t = HttpTransport::new(&addr, Duration::from_secs(30));
        let (tok, ..) = token_and_addr();
        let r = registry_writer::Transport::send(&t, &registry_writer::Request { method: "GET", path: format!("/v1/registry/proposals/{pid}"), bearer: &tok, idempotency_key: None, body: None }).unwrap();
        let chs = r.body["changes"].as_array().cloned().unwrap_or_default();
        println!("read-back: origin={} state={} changes={} kinds={:?}", r.body["proposal"]["origin"], r.body["proposal"]["state"], chs.len(), chs.iter().map(|c| c["kind"].clone()).collect::<Vec<_>>());
        assert_eq!(r.body["proposal"]["origin"], "auto_detect");
        let changelog = chs.iter().find_map(|c| c["docs"]["changelog"].as_str()).unwrap_or("");
        assert!(changelog.starts_with("Solo propuesta"), "the dossier ES changelog travels: {changelog}");
    } else {
        assert!(proof.outcome.starts_with("not_announced:"));
        println!("record keeps the dossier: {}", proof.record()["dossier"]["es"]["description"].as_str().map_or(0, str::len) > 100);
    }
}

#[test]
#[ignore = "needs the local stack"]
fn live_proven_candidate_is_announced_with_its_dossier() {
    go("estado_pqr_attempt1", true);
}

#[test]
#[ignore = "needs the local stack"]
fn live_non_improving_candidate_is_not_announced() {
    go("estado_pqr_noop", false);
}
