//! ART2 offline: a read-only tool link is proven and announced; a tighten-only policy draft of a human-owned policy is proven natively and
//! recorded `needs_owner_ack` (never announced). Scripted agent-core, REAL `build_suite.py` and `judge_story.py`.
mod common;
use common::{REG_TOKEN, cfg, finding};
use reasoning::catalog::Catalog;
use reasoning::dossier::{Labels, Runtime};
use registry_writer::eval::EvalOptions;
use registry_writer::proof::{MemoryProofStore, ProofInput, PythonScripts, announce_submission, prove};
use registry_writer::{MemoryStore, Reply, Request, Transport, TransportError, Via, Writer};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// A scripted registry whose scorer passes every guard and passes a finding case only when the draft carries the fix.
struct Core {
    drafts: RefCell<BTreeMap<String, Vec<Value>>>,
    n: RefCell<u32>,
    fixes: fn(&[Value]) -> bool,
}

impl Transport for Core {
    fn send(&self, req: &Request) -> Result<Reply, TransportError> {
        assert_eq!(req.bearer.reveal(), REG_TOKEN);
        let p = req.path.as_str();
        let ok = |b: Value| Ok(Reply { status: 200, body: b });
        match (req.method, p) {
            ("GET", _) if p.starts_with("/v1/registry/entities/agent/") => ok(json!({"content": {"id": "x", "metrics": []}})),
            ("POST", "/v1/registry/proposals") => {
                *self.n.borrow_mut() += 1;
                let id = format!("prp_{}", self.n.borrow());
                self.drafts.borrow_mut().insert(id.clone(), vec![]);
                Ok(Reply { status: 201, body: json!({"proposal_id": id, "rev": 0, "state": "draft"}) })
            }
            ("PUT", _) if p.ends_with("/draft") => {
                let id = p.split('/').nth(4).unwrap().to_string();
                self.drafts.borrow_mut().insert(id, req.body.as_ref().unwrap()["changes"].as_array().unwrap().clone());
                ok(json!({"rev": 1}))
            }
            ("POST", _) if p.ends_with("/validate") => ok(json!({"violations": [], "candidate_hash": "h"})),
            ("POST", _) if p.ends_with("/freeze") => ok(json!({"candidate_hash": "h"})),
            ("GET", _) if p.starts_with("/v1/registry/proposals/prp_") => ok(json!({"proposal": {"state": "draft", "origin": "manual", "title": "[improvement-engine] [proof-scratch] evaluation x"}})),
            ("POST", _) if p.ends_with("/evaluate") => {
                let id = p.split('/').nth(4).unwrap().to_string();
                let d = self.drafts.borrow()[&id].clone();
                let suite = d.iter().find(|c| c["kind"] == "eval_suite").unwrap()["content"].clone();
                let changes: Vec<Value> = d.iter().filter(|c| c["kind"] != "eval_suite").cloned().collect();
                let fixed = (self.fixes)(&changes);
                let cases: Vec<(String, bool)> = suite["scenarios"].as_array().unwrap().iter().map(|s| {
                    let sid = s["id"].as_str().unwrap().to_string();
                    let pass = sid.starts_with("guard-") || fixed;
                    (sid, pass)
                }).collect();
                let all = cases.iter().all(|c| c.1);
                let scen: serde_json::Map<String, Value> = cases.iter().map(|(c, p)| (c.clone(), json!(*p))).collect();
                let results: Vec<Value> = cases.iter().filter(|c| !c.1).map(|c| json!({"run": "cand_on_new", "scenario_id": c.0, "score": {"passed": false, "failures": ["expected outcome escalated, got resolved"]}})).collect();
                let items: Vec<Value> = cases.iter().map(|c| json!({"metric_id": format!("scenario/{}", c.0), "phase": "gate", "passed": c.1, "value": 1.0})).collect();
                let rep = json!({"verdict": if all { "pass" } else { "fail" }, "items": items, "runs": {"cand_on_new": {"scenarios": scen}}, "results": results});
                if all { ok(rep) } else { Ok(Reply { status: 409, body: json!({"code": "gate_failed", "payload": rep}) }) }
            }
            _ => panic!("unexpected request {} {}", req.method, req.path),
        }
    }
}

fn scripts() -> PythonScripts {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let work = std::env::temp_dir().join("art2-proof-test");
    std::fs::create_dir_all(&work).unwrap();
    PythonScripts { python: vec!["python".into()], script_dir: root.join("scripts/regression"), work, env: vec![] }
}

fn no_sleep(_: Duration) {}

fn go(core: &Core, comp: &reasoning::patch::Compiled) -> registry_writer::proof::Proof {
    let mem = MemoryStore::new();
    let w = Writer::new(cfg(Via::RegistryApi), core, &mem);
    let fi = finding();
    let inp = ProofInput { finding: &fi, compiled: comp, attempts: vec![], base_artifact: None, labels: Labels { runtime: Runtime::Real, ..Default::default() }, doubles: json!([]), rubric: json!({"total": 17, "max": 24}) };
    let opts = EvalOptions { infra_retries: 0, backoff: Duration::from_secs(0), sleep: no_sleep };
    prove(&w, &scripts(), &MemoryProofStore::default(), &opts, &inp)
}

#[test]
fn a_read_only_tool_link_is_proven_fails_on_base_and_announced_with_the_flow_agent_tool_and_suite() {
    let comp = link_compiled(&Catalog::bundled());
    let core = Core { drafts: Default::default(), n: Default::default(), fixes: |c| c.iter().any(|x| x["kind"] == "flow" && x["content"]["nodes"].to_string().contains("eng_link_leer_pqr_cliente")) };
    let p = go(&core, &comp);
    assert_eq!((p.announce, p.verdict.as_str()), (true, "regression_suite_proven"), "{} {}", p.outcome, p.story["reason"]);
    assert_eq!(p.story["base"]["failed_cases"].as_array().unwrap().len(), 6);
    let sub = announce_submission(&finding(), &comp, &p);
    let kinds: Vec<&str> = sub.changes.iter().map(|c| c["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["flow", "agent", "tool", "eval_suite"]);
    assert!(p.dossier["es"]["description"].as_str().unwrap().contains("herramienta existente"));
}

#[test]
fn a_tighten_only_draft_of_a_human_owned_policy_is_proven_but_needs_the_owner_and_is_never_announced() {
    let h = reasoning::mapping::human_owned(&finding_policy()).unwrap();
    let comp = reasoning::patch::compile_policy_tighten(&Catalog::bundled(), &h.policy).unwrap();
    let core = Core { drafts: Default::default(), n: Default::default(), fixes: |c| c.iter().any(|x| x["kind"] == "policy") };
    let p = go(&core, &comp);
    assert_eq!((p.announce, p.outcome.as_str(), p.verdict.as_str()), (false, "needs_owner_ack", "regression_suite_proven"), "{}", p.story["reason"]);
    assert!(p.suite.is_none());
    assert_eq!(p.story["base"]["failed_cases"].as_array().unwrap().len(), 6);
    assert_eq!((p.dossier["announce"].as_bool(), p.dossier["announce_reason"].as_str()), (Some(false), Some("needs_owner_ack")));
}

fn stage(n: i64, d: i64) -> Value {
    let r = n as f64 / d as f64;
    json!({"numerator": n, "denominator": d, "rate": r, "baseline_rate": 0.4, "diff": r - 0.4, "p": 0.0})
}

fn mk(metric: &str, dims: Value) -> reasoning::Finding {
    let sig = json!({"metric": metric, "dims": dims, "status": "corroborated", "reason": "replicated", "direction": "up", "claim": "association", "discovery": stage(150, 200), "holdout": stage(120, 160), "r2": {"status": "replicated"}, "p_adj": 0.0});
    let rep = json!({"semantics": "claude-standin", "cells_explored": 12, "signals": [sig], "discards": []});
    reasoning::Finding::from_report(&rep, reasoning::Source::Synthetic).unwrap().0.remove(0)
}

fn finding_policy() -> reasoning::Finding {
    mk("E2", json!({"reason_code": "policy:escalamiento-disputa-monto"}))
}

fn link_compiled(cat: &Catalog) -> reasoning::patch::Compiled {
    use reasoning::roles::{Alt, Opportunity};
    let f = mk("A5", json!({"agent": "copiloto-asesor", "tool": "leer_pqr_cliente"}));
    let row = reasoning::mapping::map_finding(&f).unwrap();
    let o = Opportunity { id: "h_1".into(), target_ref: "tool_link:consultas/leer_pqr_cliente".into(), mechanism_class: "missing_tool".into(), hypothesis: "h".into(), claimed_rate: 0.75, falsifiers: vec!["f".into()], alternatives: vec![Alt { kind: "do_nothing".into(), why_not: "x".into() }] };
    reasoning::patch::compile(cat, &f, &row, &o, &json!({"kind": "link_tool", "edge_id": "consultar.ok", "rationale": "Add the read.", "alternatives": [], "uncertainty": "Association only."})).unwrap()
}
