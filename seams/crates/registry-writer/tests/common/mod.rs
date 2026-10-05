#![allow(dead_code)]
//! Scripted transport and fixtures for the offline tests. Tokens are fake strings; nothing here touches a network.
use core_client::authorizer::Jws;
use reasoning::catalog::Artifact;
use reasoning::finding::{Finding, Source};
use reasoning::patch::Compiled;
use registry_writer::{Environment, Reply, Request, Submission, Transport, TransportError};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

pub const REG_TOKEN: &str = "fake.registry.token";
pub const RUN_TOKEN: &str = "fake.run.token";

pub type Step = (&'static str, Result<Reply, TransportError>);

#[derive(Debug, Clone)]
pub struct Logged {
    pub method: String,
    pub path: String,
    pub idem: Option<String>,
    pub body: Option<Value>,
    pub bearer_is_run_token: bool,
}

/// Replies come from a queue; every request must match the next expected `"METHOD /path-prefix"`.
#[derive(Default)]
pub struct Script {
    pub queue: RefCell<VecDeque<Step>>,
    pub log: RefCell<Vec<Logged>>,
}

impl Script {
    pub fn new(steps: Vec<Step>) -> Script {
        Script { queue: RefCell::new(steps.into()), log: RefCell::new(vec![]) }
    }
    pub fn push(&self, s: Step) {
        self.queue.borrow_mut().push_back(s);
    }
    pub fn requests(&self) -> Vec<(String, String)> {
        self.log.borrow().iter().map(|l| (l.method.clone(), l.path.clone())).collect()
    }
    pub fn count(&self, method: &str, prefix: &str) -> usize {
        self.log.borrow().iter().filter(|l| l.method == method && l.path.starts_with(prefix)).count()
    }
    pub fn remaining(&self) -> usize {
        self.queue.borrow().len()
    }
}

impl Transport for Script {
    fn send(&self, req: &Request) -> Result<Reply, TransportError> {
        let (want, reply) = self.queue.borrow_mut().pop_front().unwrap_or_else(|| panic!("unexpected request {} {}", req.method, req.path));
        let got = format!("{} {}", req.method, req.path);
        assert!(got.starts_with(want), "expected {want}, got {got}");
        self.log.borrow_mut().push(Logged {
            method: req.method.into(),
            path: req.path.clone(),
            idem: req.idempotency_key.map(str::to_string),
            body: req.body.clone(),
            bearer_is_run_token: req.bearer.reveal() == RUN_TOKEN,
        });
        reply
    }
}

pub fn ok(body: Value) -> Result<Reply, TransportError> {
    Ok(Reply { status: 200, body })
}
pub fn created(body: Value) -> Result<Reply, TransportError> {
    Ok(Reply { status: 201, body })
}
pub fn status(code: u16, problem: &str) -> Result<Reply, TransportError> {
    Ok(Reply { status: code, body: json!({"code": problem, "status": code, "detail": "free text must never be copied"}) })
}
pub fn reg_token() -> Jws {
    Jws::new(REG_TOKEN.into())
}
pub fn run_token() -> Jws {
    Jws::new(RUN_TOKEN.into())
}

pub const BASE_ES: &str = "Eres el copiloto. Respondes SOLO al asesor. Solo lees: no modificas nada. Resume la consulta en una frase.";

/// What the registry serves for the prompt `p/copiloto` at `version`.
pub fn entity(version: &str, es: &str) -> Value {
    json!({"ref": {"kind": "prompt", "id": "p/copiloto", "version": version},
           "content": {"id": "p/copiloto", "version": version, "locales": {"es": es, "pt": "Voce e o copiloto."}, "model_profile": "perfil-generacion@1"}})
}

pub fn live_artifact() -> Artifact {
    registry_writer::baseline::parse_entity(&entity("1.0.0", BASE_ES), vec![]).unwrap()
}

pub fn patch_change(version: &str) -> Value {
    json!({"kind": "prompt",
           "content": {"id": "p/copiloto", "version": version, "locales": {"es": format!("{BASE_ES} Antes de repetir una consulta revisa el historial."), "pt": "Voce e o copiloto."}, "model_profile": "perfil-generacion@1"},
           "docs": {"description": "[improvement-engine] prompt:p/copiloto repeated_lookup", "rationale": "the advisor repeats the same lookup in 77 of 100 cases"}})
}

pub fn compiled_patch() -> Compiled {
    Compiled {
        kind: "patch".into(),
        target_ref: "prompt:p/copiloto".into(),
        agent_id: "copiloto-asesor".into(),
        changes: vec![patch_change("1.0.1")],
        diff: vec![],
        base_digest: live_artifact().digest(),
        cascade: vec![],
        edit_chars: 60,
        edit_budget: 300,
        human_items: vec![],
        expected_effect: Value::Null,
        rationale: "advisors repeat the same recent-charges lookup in 154 of 200 dispute cases; ask the copilot to reuse the first answer".into(),
        uncertainty: String::new(),
    }
}

/// A corroborated finding from the REAL L1 sensor over the synthetic grid.
pub fn finding() -> Finding {
    let report: Value = serde_json::from_str(&steps::cells::run(&reasoning::testkit::synthetic_cells_ndjson()).unwrap()).unwrap();
    let (mut f, _) = Finding::from_report(&report, Source::Synthetic).unwrap();
    f.remove(0)
}

pub fn submission() -> Submission {
    Submission::new(&finding(), &compiled_patch())
}

pub fn cfg(via: registry_writer::Via) -> registry_writer::Config {
    let mut c = registry_writer::Config::new(via, Environment::LocalStack, reg_token());
    c.run_token = Some(run_token());
    c
}

pub fn proposal_detail(rev: u64, state: &str, n_changes: usize) -> Value {
    json!({"proposal": {"proposal_id": "prp_1", "rev": rev, "state": state}, "changes": (0..n_changes).map(|_| patch_change("1.0.1")).collect::<Vec<_>>(), "last_eval": null, "review": null})
}

pub fn valid() -> Value {
    json!({"violations": [], "candidate_hash": "h", "auto_bumped": []})
}

pub fn created_body() -> Value {
    json!({"proposal_id": "prp_1", "rev": 0, "state": "draft", "agent_id": "copiloto-asesor"})
}

/// The five requests of a clean direct delivery.
pub fn direct_steps() -> Vec<Step> {
    vec![
        ("GET /v1/registry/entities/prompt/p/copiloto", ok(entity("1.0.0", BASE_ES))),
        listing_empty(),
        ("POST /v1/registry/proposals", created(created_body())),
        ("PUT /v1/registry/proposals/prp_1/draft", ok(json!({"proposal_id": "prp_1", "rev": 1, "state": "draft"}))),
        ("POST /v1/registry/proposals/prp_1/validate", ok(valid())),
        ("GET /v1/registry/proposals/prp_1", ok(proposal_detail(1, "draft", 1))),
    ]
}

pub fn listing_empty() -> Step {
    ("GET /v1/registry/proposals?agent_id=", ok(json!({"items": [], "total": 0})))
}

pub fn none_of(map: &BTreeMap<String, String>) -> bool {
    map.is_empty()
}
