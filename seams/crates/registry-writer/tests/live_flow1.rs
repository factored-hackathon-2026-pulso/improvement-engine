//! LIVE FLOW1: the MODEL picks a position and a preset of a flow edit (validator for the radicado; ask on the dispute flow); same stack and env as live_flow1.
//! (history: LIVE ART3: the MODEL drives the tool link (Scout and Builder mimo flash, independent Verifier mimo pro, real gateway) on YOUR OWN stack:
//! finding -> mapping row `tool_peers_gap` -> Scout -> Verifier -> Builder picks `edge_id` -> compiler -> independent link review -> proof
//! (fails on base, passes on candidate) -> announced as an auto_detect draft. The policy finding takes the deterministic path (no model call:
//! the hypothesis note and the tighten draft come from structured params), is proven natively and recorded `needs_owner_ack`.
//!
//! ```text
//! PULSO_LLM_GATEWAY=enabled PULSO_LLM_GATEWAY_ADDR=127.0.0.1:8182 PULSO_LLM_GATEWAY_KEY=<set by the launcher, never printed> \
//! PULSO_LLM_GATEWAY_MODEL=xiaomi/mimo-v2.6-flash PULSO_LLM_GATEWAY_VERIFIER_MODEL=xiaomi/mimo-v2.6-pro \
//! PULSO_DEV_STACK_DIR=<worktree>/.dev-stack/battery-pulso-art2 PULSO_STACK_ADDR=127.0.0.1:8082 \
//!   cargo test -j 1 -p registry-writer --test live_flow1 -- --ignored --nocapture --test-threads 1
//! ```
use core_client::authorizer::Jws;
use reasoning::catalog::Catalog;
use reasoning::dossier::{Labels, Runtime};
use reasoning::finding::{Finding, Source};
use reasoning::pipeline::{Opts, reason_candidate};
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
    let work = std::env::temp_dir().join("pulso-flow1-live");
    std::fs::create_dir_all(&work).unwrap();
    PythonScripts { python: py.split_whitespace().map(str::to_string).collect(), script_dir: root().join("scripts/regression"), work, env: vec![] }
}

fn live_catalog(w: &Writer) -> Catalog {
    let mut c = Catalog::bundled();
    for (kind, id) in [("flow", "consulta-pqr"), ("agent", "consultas"), ("flow", "disputa-cargo"), ("agent", "disputas")] {
        c.put_entity(kind, id, w.fetch_content(kind, id).unwrap_or_else(|| panic!("live {kind} {id} cannot be read")));
    }
    c.label = "live-registry".into();
    c
}

fn story(p: &registry_writer::proof::Proof) {
    println!("proof: outcome={} verdict={} announce={} reason={}", p.outcome, p.verdict, p.announce, p.story["reason"]);
    println!("base failed finding cases={} guards failed={}", p.story["base"]["failed_cases"].as_array().map_or(0, Vec::len), p.story["base"]["guards_failed"].as_array().map_or(0, Vec::len));
    let gi = p.story["gate_items"].as_array().cloned().unwrap_or_default();
    println!("GateItems: {} total, {} passed", gi.len(), gi.iter().filter(|g| g["passed"] == true).count());
}

fn run_live(f: Finding, target: &str, tag: &str) {
    let (tok, addr) = admin();
    let long = HttpTransport::new(&addr, Duration::from_secs(900));
    let store = FileStore::new(std::env::temp_dir().join(format!("pulso-flow1-receipts-{tag}-{}.json", std::process::id())));
    let mut cfg = Config::new(Via::RegistryApi, Environment::LocalStack, tok);
    cfg.credential = "operator-declared stand-in credential (not the engine builder principal)";
    let w = Writer::new(cfg, &long, &store);
    let cat = live_catalog(&w);
    let (mut best, mut calls, mut cost) = (None, 0u64, 0f64);
    for _ in 0..3 {
        let ports = reasoning::live::ports_from_env(&|k| std::env::var(k).ok()).expect("live gateway env (see the module docs)");
        let r = reason_candidate(&cat, &f, &ports, &Opts::default(), Some(target));
        calls += r.metering["calls"].as_u64().unwrap_or(0);
        cost += r.metering["cost_usd"].as_f64().unwrap_or(0.0);
        println!("attempt: status={} reason={} stage={} calls={} tokens_in={} tokens_out={} cost_usd={} attempts={} detail={}", r.status, r.reason, r.stage, r.metering["calls"], r.metering["tokens_in"], r.metering["tokens_out"], r.metering["cost_usd"], r.metering["attempts"], r.detail);
        if r.status == "proposed" {
            best = Some(r);
            break;
        }
    }
    println!("total model calls: {calls}, total cost_usd: {cost:.6}");
    let r = best.expect("the model produced no compiled flow edit in 3 attempts");
    assert!(r.calls.iter().all(|c| c["real"] == true) && r.doubles.is_empty(), "every call was answered by the real gateway");
    let c = r.compiled_raw.as_ref().unwrap();
    println!("model chose {} ; flow review: {}", c.diff[0]["position"], r.verification.as_ref().unwrap()["flow_review"]);
    let inp = ProofInput { finding: &f, compiled: c, attempts: vec![], base_artifact: None, labels: Labels { runtime: Runtime::Real, ..Default::default() }, doubles: json!(r.doubles), rubric: r.rubric.clone().unwrap_or(Value::Null) };
    let proofs = FileProofStore::new(std::env::temp_dir().join(format!("pulso-flow1-proofs-{tag}-{}.json", std::process::id())));
    let p = prove(&w, &scripts(), &proofs, &EvalOptions::default(), &inp);
    story(&p);
    println!("coverage: {}", p.story["coverage"]);
    if !p.announce {
        panic!("not announced: {} / {}", p.outcome, p.story["reason"]);
    }
    let o = w.deliver(&announce_submission(&f, c, &p));
    println!("announced delivery: status={} proposal={} changes={}", o.to_json()["status"], o.to_json()["proposal_id"], o.changes);
    assert!(o.delivered(), "{}", o.to_json());
}

#[test]
#[ignore = "needs the own local stack and the local gateway (real models)"]
fn live_validator_for_the_radicado_is_proven_and_announced() {
    run_live(mk("M4", json!({"category": "Technical"})), "flow_edit:consulta-pqr/add_validator", "val");
}

#[test]
#[ignore = "needs the own local stack and the local gateway (real models)"]
fn live_ask_on_the_dispute_flow_is_proven_and_announced() {
    run_live(mk("P_ASSIST_ESCALATION", json!({"case_type": "undue_charge", "channel": "web_chat"})), "flow_edit:disputa-cargo/insert_ask", "ask");
}
