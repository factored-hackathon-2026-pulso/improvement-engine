//! W11 offline TDD: "evaluate before announce" against a scripted agent-core (`Core`) and the REAL Python judge
//! (`scripts/regression/judge_story.py`, stdlib only) over the committed REG1 bundle of the `t/estado_pqr` finding.
//! Verdicts covered: regression_suite_proven, non_discriminating, not_fixed, guard_regressed, infra_failed, suite_refused; retries,
//! forbidden verbs, manual-origin evaluation drafts vs the auto_detect announced proposal, idempotent replay, no secrets.
mod common;
use common::{REG_TOKEN, cfg, finding};
use reasoning::catalog::Catalog;
use reasoning::dossier::{Labels, Runtime};
use reasoning::patch::Compiled;
use registry_writer::eval::EvalOptions;
use registry_writer::guard::allowed;
use registry_writer::proof::{MemoryProofStore, ProofInput, ProofStore, PythonScripts, Scripts, SuiteError, announce_submission, prove};
use registry_writer::{MemoryStore, Reply, Request, Transport, TransportError, Via, Writer};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn bundle() -> Value {
    serde_json::from_str(&std::fs::read_to_string(root().join("scripts/regression/results/reg-consultas-ffbb5234.bundle.json")).unwrap()).unwrap()
}

fn candidate(name: &str) -> Vec<Value> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(root().join(format!("scripts/regression/fixtures/candidates/{name}.json"))).unwrap()).unwrap();
    v["changes"].as_array().unwrap().clone()
}

fn compiled(changes: Vec<Value>) -> Compiled {
    Compiled {
        kind: "patch".into(),
        target_ref: "template:t/estado_pqr".into(),
        agent_id: "consultas".into(),
        changes,
        diff: vec![],
        base_digest: Catalog::bundled().get("template:t/estado_pqr").unwrap().digest(),
        cascade: vec![],
        edit_chars: 90,
        edit_budget: 300,
        human_items: vec![],
        expected_effect: Value::Null,
        rationale: "the status sentence names no state; add the status placeholder".into(),
        uncertainty: String::new(),
    }
}

// ---------------------------------------------------------------------------------------------------- scripted agent-core
struct Logged {
    method: String,
    path: String,
    idem: Option<String>,
    body: Option<Value>,
}

/// A tiny agent-core registry: proposals, drafts, validate, freeze, evaluate. `native` decides which scenarios its scorer passes.
struct Core {
    log: RefCell<Vec<Logged>>,
    drafts: RefCell<BTreeMap<String, Vec<Value>>>,
    n: Cell<u32>,
    evals: Cell<u32>,
    /// Replies injected for a route suffix, consumed in order.
    inject: RefCell<Vec<(String, Reply)>>,
    native: Box<dyn Fn(&[Value]) -> Vec<(String, bool)>>,
    /// Entity ids the registry answers 404 for (a closure that cannot be read).
    missing: RefCell<Vec<String>>,
}

fn reply(status: u16, body: Value) -> Result<Reply, TransportError> {
    Ok(Reply { status, body })
}

impl Core {
    fn new(native: impl Fn(&[Value]) -> Vec<(String, bool)> + 'static) -> Core {
        Core { log: Default::default(), drafts: Default::default(), n: Cell::new(0), evals: Cell::new(0), inject: Default::default(), native: Box::new(native), missing: Default::default() }
    }
    fn inject(&self, suffix: &str, r: Reply) {
        self.inject.borrow_mut().push((suffix.into(), r));
    }
    fn count(&self, method: &str, suffix: &str) -> usize {
        self.log.borrow().iter().filter(|l| l.method == method && l.path.ends_with(suffix)).count()
    }
    fn report(&self, changes: &[Value]) -> Value {
        let cases = (self.native)(changes);
        let pass = cases.iter().all(|(_, p)| *p);
        let scen: serde_json::Map<String, Value> = cases.iter().map(|(c, p)| (c.clone(), json!(*p))).collect();
        let results: Vec<Value> = cases.iter().filter(|(_, p)| !*p).map(|(c, _)| json!({"run": "cand_on_new", "scenario_id": c, "score": {"passed": false, "failures": ["expected outcome resolved, got escalated"]}})).collect();
        let items: Vec<Value> = cases.iter().map(|(c, p)| json!({"metric_id": format!("scenario/{c}"), "phase": "gate", "passed": p, "value": if *p { 1.0 } else { 0.0 }})).collect();
        json!({"verdict": if pass { "pass" } else { "fail" }, "items": items, "runs": {"cand_on_new": {"scenarios": scen}}, "results": results, "detail": null})
    }
}

impl Transport for Core {
    fn send(&self, req: &Request) -> Result<Reply, TransportError> {
        assert_eq!(req.bearer.reveal(), REG_TOKEN);
        self.log.borrow_mut().push(Logged { method: req.method.into(), path: req.path.clone(), idem: req.idempotency_key.map(str::to_string), body: req.body.clone() });
        {
            let mut inj = self.inject.borrow_mut();
            if let Some(i) = inj.iter().position(|(s, _)| req.path.ends_with(s.as_str())) {
                return Ok(inj.remove(i).1);
            }
        }
        let p = req.path.as_str();
        match (req.method, p) {
            ("GET", "/v1/registry/entities/agent/consultas") => reply(200, json!({"content": {"id": "consultas", "metrics": [{"id": "resolution_rate", "role": "gate"}, {"id": "note", "role": "info"}]}})),
            ("GET", _) if p.starts_with("/v1/registry/entities/") => {
                let (kind, id) = p["/v1/registry/entities/".len()..].split_once('/').unwrap();
                if self.missing.borrow().iter().any(|m| m == id) {
                    return reply(404, json!({"code": "not_found"}));
                }
                let content = match kind {
                    "agent" => json!({"id": id, "metrics": []}),
                    "template" => json!({"id": id, "version": "1.0.0", "locales": {"es": "texto del donante", "pt": "texto do doador"}}),
                    _ => json!({"id": id, "version": "1.0.0"}),
                };
                reply(200, json!({"ref": {"kind": kind, "id": id, "version": "1.0.0"}, "content": content}))
            }
            ("GET", _) if p.starts_with("/v1/registry/proposals?agent_id=") => reply(200, json!({"items": [], "total": 0})),
            ("POST", "/v1/registry/proposals") => {
                self.n.set(self.n.get() + 1);
                let id = format!("prp_{}", self.n.get());
                self.drafts.borrow_mut().insert(id.clone(), vec![]);
                reply(201, json!({"proposal_id": id, "rev": 0, "state": "draft", "origin": req.body.as_ref().unwrap()["origin"]}))
            }
            ("PUT", _) if p.ends_with("/draft") => {
                let id = p.split('/').nth(4).unwrap().to_string();
                self.drafts.borrow_mut().insert(id, req.body.as_ref().unwrap()["changes"].as_array().unwrap().clone());
                reply(200, json!({"rev": 1}))
            }
            ("GET", _) if p.starts_with("/v1/registry/proposals/prp_") => {
                let id = p.rsplit('/').next().unwrap();
                let n = self.drafts.borrow().get(id).map_or(0, Vec::len);
                reply(200, json!({"proposal": {"proposal_id": id, "rev": 1, "state": "draft"}, "changes": (0..n).map(|_| json!({})).collect::<Vec<_>>()}))
            }
            ("POST", _) if p.ends_with("/validate") => reply(200, json!({"violations": [], "candidate_hash": "h"})),
            ("POST", _) if p.ends_with("/freeze") => reply(200, json!({"candidate_hash": "h"})),
            ("POST", _) if p.ends_with("/evaluate") => {
                self.evals.set(self.evals.get() + 1);
                let id = p.split('/').nth(4).unwrap().to_string();
                let drafted = self.drafts.borrow()[&id].clone();
                // verified live: a brand-new agent cannot be evaluated in a release without the donor's settings (500)
                if drafted.iter().any(|c| c["kind"] == "agent") && !drafted.iter().any(|c| c["kind"] == "release_settings") {
                    return reply(500, json!({"code": "internal_error"}));
                }
                let changes: Vec<Value> = drafted.iter().filter(|c| c["kind"] != "eval_suite").cloned().collect();
                let rep = self.report(&changes);
                if rep["verdict"] == "pass" { reply(200, rep) } else { reply(409, json!({"code": "gate_failed", "payload": rep})) }
            }
            _ => panic!("unexpected request {} {}", req.method, req.path),
        }
    }
}

/// Native scorer: every case passes, except that a candidate whose template uses an unknown placeholder escalates in the real
/// engine (all finding cases fail) and `break_guard` breaks one guard on any candidate.
fn native(break_guard: bool) -> impl Fn(&[Value]) -> Vec<(String, bool)> + 'static {
    let b = bundle();
    let finding_ids: Vec<String> = b["finding_case_ids"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let guard_ids: Vec<String> = b["guard_case_ids"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    move |changes| {
        let bad = changes.iter().any(|c| c["content"]["locales"]["es"].as_str().is_some_and(|t| t.contains("pqr.value.estado")));
        let mut out: Vec<(String, bool)> = finding_ids.iter().map(|c| (c.clone(), !bad)).collect();
        out.extend(guard_ids.iter().enumerate().map(|(i, c)| (c.clone(), !(break_guard && !changes.is_empty() && i == 0))));
        out
    }
}

// ---------------------------------------------------------------------------------------------------- scripts
/// The committed bundle for `build_suite`, the REAL python judge.
struct Fixed {
    py: PythonScripts,
    refuse: Option<(String, String)>,
    bundle: Value,
    /// The new agent slug the proof asked the bundle for.
    asked_agent: RefCell<Option<String>>,
}

impl Scripts for Fixed {
    fn build_suite(&self, _f: &Value, _t: &str) -> Result<Value, SuiteError> {
        match &self.refuse {
            Some((c, w)) => Err(SuiteError::Refused(c.clone(), w.clone())),
            None => Ok(self.bundle.clone()),
        }
    }
    fn build_suite_for(&self, f: &Value, t: &str, new_agent: Option<&str>) -> Result<Value, SuiteError> {
        *self.asked_agent.borrow_mut() = new_agent.map(str::to_string);
        self.build_suite(f, t)
    }
    fn judge(&self, input: &Value) -> Result<Value, String> {
        self.py.judge(input)
    }
    fn sample(&self, input: &Value) -> Result<Value, String> {
        self.py.sample(input)
    }
}

fn scripts_for(bundle: Value, env: Vec<(String, String)>) -> Fixed {
    Fixed { py: PythonScripts { python: vec!["python".into()], script_dir: root().join("scripts/regression"), work: std::env::temp_dir(), env }, refuse: None, bundle, asked_agent: Default::default() }
}

fn scripts() -> Fixed {
    scripts_for(bundle(), vec![])
}

thread_local! {
    static SLEPT: Cell<u32> = const { Cell::new(0) };
}
fn no_sleep(_: Duration) {
    SLEPT.with(|s| s.set(s.get() + 1));
}
fn opts() -> EvalOptions {
    SLEPT.with(|s| s.set(0));
    EvalOptions { infra_retries: 4, backoff: Duration::from_secs(20), sleep: no_sleep }
}

fn input<'a>(f: &'a registry_writer::Submission, fi: &'a reasoning::Finding, c: &'a Compiled) -> ProofInput<'a> {
    let _ = f;
    ProofInput { finding: fi, compiled: c, attempts: vec![], base_artifact: None, labels: Labels { runtime: Runtime::Real, ..Default::default() }, doubles: json!([]), rubric: json!({"total": 17, "max": 20}) }
}

fn run(core: &Core, comp: &Compiled, attempts: Vec<Vec<Value>>, sc: &Fixed, store: &dyn ProofStore) -> registry_writer::proof::Proof {
    let mem = MemoryStore::new();
    let w = Writer::new(cfg(Via::RegistryApi), core, &mem);
    let fi = finding();
    let sub = registry_writer::Submission::new(&fi, comp);
    let mut inp = input(&sub, &fi, comp);
    inp.attempts = attempts;
    prove(&w, sc, store, &opts(), &inp)
}

// ---------------------------------------------------------------------------------------------------- tests
#[test]
fn a_proven_proposal_is_announced_with_the_verdict_story_and_the_dossier_text() {
    let core = Core::new(native(false));
    let comp = compiled(candidate("estado_pqr_attempt1"));
    let p = run(&core, &comp, vec![], &scripts(), &MemoryProofStore::default());
    assert_eq!((p.announce, p.outcome.as_str(), p.verdict.as_str()), (true, "announced", "regression_suite_proven"), "{}", p.story["reason"]);
    assert_eq!(p.story["suite_is_regression_suite"], true);
    assert_eq!(p.story["base"]["failed_cases"].as_array().unwrap().len(), 8, "the base fails the 8 finding cases by the wording probe");
    assert_eq!(p.story["attempts"][0]["verdict"], "pass");
    assert!(!p.story["gate_items"].as_array().unwrap().is_empty(), "the real GateItems are carried");
    assert_eq!(p.dossier["announce"], true);
    assert!(p.dossier["es"]["sections"]["result"].as_str().unwrap().contains("Suite de regresión probada"));
    assert_eq!(p.eval_proposals.len(), 2, "one base and one candidate evaluation draft");
    // the evaluation drafts are MANUAL origin and carry the suite; freeze and evaluate each ran once per run
    let log = core.log.borrow();
    let creates: Vec<&Logged> = log.iter().filter(|l| l.method == "POST" && l.path == "/v1/registry/proposals").collect();
    assert!(creates.iter().all(|l| l.body.as_ref().unwrap()["origin"] == "manual"), "evaluation drafts never use the auto_detect quota");
    assert!(creates.iter().all(|l| l.idem.as_deref().is_some_and(|k| k.starts_with("pulso-"))));
    assert_eq!((core.count("POST", "/freeze"), core.count("POST", "/evaluate")), (2, 2));
    let puts: Vec<&Logged> = log.iter().filter(|l| l.method == "PUT").collect();
    assert_eq!(puts[0].body.as_ref().unwrap()["changes"].as_array().unwrap().len(), 1, "base draft = the suite only");
    assert_eq!(puts[0].body.as_ref().unwrap()["changes"][0]["kind"], "eval_suite");
    assert_eq!(puts[1].body.as_ref().unwrap()["changes"].as_array().unwrap().len(), 2, "candidate draft = patch + suite");
    // default thresholds were added for the agent's gate metric (agent-core refuses a suite without them)
    assert_eq!(puts[0].body.as_ref().unwrap()["changes"][0]["content"]["thresholds"]["resolution_rate"]["noise_margin"], "0");
    assert!(puts[0].body.as_ref().unwrap()["changes"][0]["content"]["thresholds"].get("note").is_none());
    for l in log.iter() {
        assert!(allowed(&l.method, &l.path), "{} {}", l.method, l.path);
        assert!(!["approve", "publish", "promote", "reject", "reopen", "revoke"].iter().any(|w| l.path.contains(w)), "{}", l.path);
    }
}

#[test]
fn only_an_announced_proof_is_delivered_as_auto_detect_with_the_dossier_text_and_the_suite() {
    let core = Core::new(native(false));
    let comp = compiled(candidate("estado_pqr_attempt1"));
    let p = run(&core, &comp, vec![], &scripts(), &MemoryProofStore::default());
    assert!(p.announce);
    let sub = announce_submission(&finding(), &comp, &p);
    assert_eq!(sub.changes.len(), 2);
    assert_eq!(sub.changes[0]["docs"]["changelog"], p.dossier["es"]["changelog"], "the changelog is the dossier's ES changelog");
    assert_eq!(sub.changes[0]["docs"]["rationale"], p.dossier["es"]["rationale"]);
    assert_eq!(sub.changes[0]["docs"]["description"], p.dossier["es"]["description"]);
    assert_eq!(sub.changes[1]["kind"], "eval_suite");
    let mem = MemoryStore::new();
    let mut c = cfg(Via::RegistryApi);
    c.check_base = false;
    let before = core.log.borrow().len();
    let o = Writer::new(c, &core, &mem).deliver(&sub);
    assert!(o.delivered(), "{}", o.to_json());
    let log = core.log.borrow();
    let create = log[before..].iter().find(|l| l.method == "POST" && l.path == "/v1/registry/proposals").unwrap();
    assert_eq!(create.body.as_ref().unwrap()["origin"], "auto_detect");
    assert_eq!(o.changes, 2);
    let put = log[before..].iter().find(|l| l.method == "PUT").unwrap().body.clone().unwrap();
    assert_eq!(put["changes"][0]["docs"]["changelog"], p.dossier["es"]["changelog"]);
}

#[test]
fn a_non_improving_candidate_is_not_announced() {
    let core = Core::new(native(false));
    let comp = compiled(candidate("estado_pqr_noop"));
    let p = run(&core, &comp, vec![], &scripts(), &MemoryProofStore::default());
    assert_eq!((p.announce, p.outcome.as_str()), (false, "not_announced:not_fixed"), "{}", p.story["reason"]);
    assert_eq!(p.dossier["announce"], false);
    assert!(p.suite.is_none());
    assert!(p.dossier["es"]["description"].as_str().unwrap().contains("NO SE ANUNCIA"));
    assert!(p.record()["dossier"]["es"]["description"].is_string(), "the dossier is kept in the record, never dropped");
}

#[test]
fn a_placeholder_the_engine_cannot_resolve_is_not_fixed() {
    let core = Core::new(native(false));
    let comp = compiled(candidate("estado_pqr_badplaceholder"));
    let p = run(&core, &comp, vec![], &scripts(), &MemoryProofStore::default());
    assert_eq!(p.outcome, "not_announced:not_fixed");
    assert_eq!(p.story["attempts"][0]["native_verdict"], "fail");
}

#[test]
fn a_first_attempt_that_fails_then_a_second_that_passes_tells_the_story_and_is_announced() {
    let core = Core::new(native(false));
    let bad = candidate("estado_pqr_badplaceholder");
    let good = candidate("estado_pqr_attempt1");
    let comp = compiled(good.clone());
    let p = run(&core, &comp, vec![bad, good], &scripts(), &MemoryProofStore::default());
    assert_eq!(p.outcome, "announced", "{}", p.story["reason"]);
    assert_eq!(p.story["attempts"].as_array().unwrap().len(), 2);
    assert!(p.story["story_text"]["es"].as_str().unwrap().contains("intento 1 fallo"));
    assert!(p.dossier["es"]["sections"]["result"].as_str().unwrap().contains("intento 1 falló"));
    assert_eq!(p.eval_proposals.len(), 3);
}

#[test]
fn a_base_that_already_passes_is_non_discriminating_and_no_candidate_is_evaluated() {
    let core = Core::new(native(false));
    let comp = compiled(candidate("estado_pqr_attempt1"));
    let mem = MemoryStore::new();
    let w = Writer::new(cfg(Via::RegistryApi), &core, &mem);
    let fi = finding();
    let sub = registry_writer::Submission::new(&fi, &comp);
    // the live base already renders the state: the wording probes pass on the base
    let mut live = Catalog::bundled().get("template:t/estado_pqr").unwrap().clone();
    live.locales.insert("es".into(), "Estado: {{ facts.pqr.value.status }}".into());
    live.locales.insert("pt".into(), "Estado: {{ facts.pqr.value.status }}".into());
    let mut inp = input(&sub, &fi, &comp);
    inp.base_artifact = Some(&live);
    let p = prove(&w, &scripts(), &MemoryProofStore::default(), &opts(), &inp);
    assert_eq!((p.announce, p.outcome.as_str()), (false, "not_announced:non_discriminating"), "{}", p.story["reason"]);
    assert_eq!(core.count("POST", "/evaluate"), 1, "only the base was evaluated");
    assert_eq!(p.dossier["es"]["sections"]["result"].as_str().unwrap().contains("NO DISCRIMINANTE"), true);
}

#[test]
fn a_candidate_that_breaks_a_guard_is_guard_regressed() {
    let core = Core::new(native(true));
    let comp = compiled(candidate("estado_pqr_attempt1"));
    let p = run(&core, &comp, vec![], &scripts(), &MemoryProofStore::default());
    assert_eq!((p.announce, p.outcome.as_str()), (false, "not_announced:guard_regressed"), "{}", p.story["reason"]);
    assert!(p.story["reason"].as_str().unwrap().contains("guard-"));
}

#[test]
fn infrastructure_failures_are_retried_a_bounded_number_of_times_then_reported_as_infra_failed() {
    let core = Core::new(native(false));
    for _ in 0..20 {
        core.inject("/evaluate", Reply { status: 200, body: json!({"verdict": "failed_infra", "detail": "HarnessUnavailable jev", "items": []}) });
    }
    let comp = compiled(candidate("estado_pqr_attempt1"));
    let store = MemoryProofStore::default();
    let p = run(&core, &comp, vec![], &scripts(), &store);
    assert_eq!((p.announce, p.outcome.as_str()), (false, "not_announced:infra_failed"));
    assert_eq!(core.count("POST", "/evaluate"), 5, "1 try + 4 retries, then stop");
    assert_eq!(SLEPT.with(Cell::get), 4);
    assert_eq!(p.story["base"]["infra_retries"].as_array().unwrap().len(), 4, "every retry is recorded");
    // inconclusive: a later job re-runs it under a fresh idempotency salt instead of replaying the dead drafts
    let first_keys: Vec<String> = core.log.borrow().iter().filter_map(|l| l.idem.clone()).collect();
    for _ in 0..0 {}
    let again = run(&core, &comp, vec![], &scripts(), &store);
    assert!(!again.replayed);
    let all: Vec<String> = core.log.borrow().iter().filter(|l| l.method == "POST" && l.path == "/v1/registry/proposals").filter_map(|l| l.idem.clone()).collect();
    let unique: std::collections::BTreeSet<&String> = all.iter().collect();
    assert_eq!(unique.len(), all.len(), "no Idempotency-Key is ever reused across tries or re-runs: {first_keys:?}");
}

#[test]
fn a_transient_infra_failure_is_retried_and_the_proof_still_succeeds() {
    let core = Core::new(native(false));
    core.inject("/evaluate", Reply { status: 200, body: json!({"verdict": "failed_infra", "detail": "HarnessUnavailable jev", "items": []}) });
    core.inject("/freeze", Reply { status: 503, body: json!({"code": "unavailable"}) });
    let comp = compiled(candidate("estado_pqr_attempt1"));
    let p = run(&core, &comp, vec![], &scripts(), &MemoryProofStore::default());
    assert_eq!(p.outcome, "announced", "{}", p.story["reason"]);
    assert_eq!(p.story["base"]["infra_retries"].as_array().unwrap().len(), 2);
}

#[test]
fn a_deterministic_refusal_is_not_retried() {
    let core = Core::new(native(false));
    core.inject("/draft", Reply { status: 422, body: json!({"code": "validation_failed", "violations": [{"rule": "REG-SCHEMA"}]}) });
    let comp = compiled(candidate("estado_pqr_attempt1"));
    let p = run(&core, &comp, vec![], &scripts(), &MemoryProofStore::default());
    assert_eq!(p.outcome, "not_announced:infra_failed");
    assert_eq!(core.count("PUT", "/draft"), 1);
    assert_eq!(SLEPT.with(Cell::get), 0);
    assert_eq!(p.story["base"]["problem"]["step"], "put_draft");
    assert_eq!(p.story["base"]["problem"]["code"], "validation_failed");
}

#[test]
fn a_refused_suite_makes_no_request_and_is_an_internal_outcome() {
    let core = Core::new(native(false));
    let mut sc = scripts();
    sc.refuse = Some(("k_below_minimum".into(), "discovery: denominator 4 < k=10".into()));
    let comp = compiled(candidate("estado_pqr_attempt1"));
    let p = run(&core, &comp, vec![], &sc, &MemoryProofStore::default());
    assert_eq!((p.announce, p.outcome.as_str()), (false, "not_announced:suite_refused"));
    assert!(core.log.borrow().is_empty());
    assert_eq!(p.story["reason"], "k_below_minimum");
    assert!(p.dossier["es"]["sections"]["result"].as_str().unwrap().contains("suite"), "{}", p.dossier["es"]["sections"]["result"]);
}

#[test]
fn a_conclusive_proof_is_replayed_without_any_request_or_subprocess() {
    let core = Core::new(native(false));
    let comp = compiled(candidate("estado_pqr_attempt1"));
    let store = MemoryProofStore::default();
    let first = run(&core, &comp, vec![], &scripts(), &store);
    let n = core.log.borrow().len();
    let mut sc = scripts();
    sc.refuse = Some(("must_not_run".into(), "x".into())); // a replay must not even call build_suite
    let again = run(&core, &comp, vec![], &sc, &store);
    assert!(again.replayed && again.announce);
    assert_eq!(again.dossier, first.dossier);
    assert_eq!(core.log.borrow().len(), n);
}

#[test]
fn the_record_carries_honest_labels_and_no_secret() {
    let core = Core::new(native(false));
    let comp = compiled(candidate("estado_pqr_attempt1"));
    let p = run(&core, &comp, vec![], &scripts(), &MemoryProofStore::default());
    let rec = p.record();
    let text = rec.to_string();
    assert!(!text.contains(REG_TOKEN) && !text.contains("Bearer"));
    assert_eq!(rec["labels"]["calibration"], "uncalibrated");
    assert_eq!(rec["labels"]["rubric"]["total"], 17);
    assert!(rec["labels"]["judge_family"].as_str().unwrap().contains("deterministic checks only"));
    assert_eq!(rec["labels"]["runtime"], "real");
    assert_eq!(rec["outcome"], "announced");
    assert!(rec["story_text"]["es"].as_str().unwrap().contains("La base falla"));
}

#[test]
fn two_candidates_of_the_same_finding_never_share_an_idempotency_key() {
    let core = Core::new(native(false));
    let store = MemoryProofStore::default();
    run(&core, &compiled(candidate("estado_pqr_noop")), vec![], &scripts(), &store);
    run(&core, &compiled(candidate("estado_pqr_attempt1")), vec![], &scripts(), &store);
    let keys: Vec<String> = core.log.borrow().iter().filter(|l| l.method == "POST" && l.path == "/v1/registry/proposals").filter_map(|l| l.idem.clone()).collect();
    let unique: std::collections::BTreeSet<&String> = keys.iter().collect();
    assert_eq!((keys.len(), unique.len()), (4, 4), "{keys:?}");
}

// ====================================================================================================== W13
// Prompt targets (native model-path assertion + binding control + harness probe through a LOCAL gateway double) and a new agent
// (clone-closure, absent base, closure copies, evaluation-only release settings). Real Python judge, real sample_probes.py.
mod w13 {
    use super::*;
    use reasoning::mapping::map_finding;
    use reasoning::patch::compile;
    use reasoning::roles::{Alt, Opportunity};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn prompt_bundle() -> Value {
        serde_json::from_str(&std::fs::read_to_string(root().join("scripts/regression/results/reg-disputas-ffbb5234.bundle.json")).unwrap()).unwrap()
    }

    fn agent_bundle() -> Value {
        serde_json::from_str(&std::fs::read_to_string(root().join("scripts/regression/results/reg-soporte-tecnico-56008268.bundle.json")).unwrap()).unwrap()
    }

    fn prompt_compiled(changes: Vec<Value>) -> Compiled {
        let mut c = compiled(changes);
        c.target_ref = "prompt:p/resumen_radicado".into();
        c.agent_id = "disputas".into();
        c.base_digest = Catalog::bundled().get("prompt:p/resumen_radicado").unwrap().digest();
        c
    }

    /// A local double of the llm-gateway `/v1/generate`: answers a follow-up sentence iff the prompt text under test names the
    /// specialist (the probe then passes), otherwise a bare confirmation. Counts the requests it served.
    fn gateway() -> (String, Arc<AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let served = Arc::new(AtomicUsize::new(0));
        let n = served.clone();
        std::thread::spawn(move || {
            for stream in l.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = vec![];
                let mut chunk = [0u8; 4096];
                let (head_end, len) = loop {
                    let k = s.read(&mut chunk).unwrap_or(0);
                    if k == 0 {
                        break (0, 0);
                    }
                    buf.extend_from_slice(&chunk[..k]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
                        let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                        break (i + 4, len);
                    }
                };
                while buf.len() < head_end + len {
                    let k = s.read(&mut chunk).unwrap_or(0);
                    if k == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..k]);
                }
                let body: Value = serde_json::from_slice(&buf[head_end.min(buf.len())..]).unwrap_or(Value::Null);
                let prompt = body["prompt"].as_str().unwrap_or("");
                let pt = prompt.contains("Redija");
                let out = match (prompt.contains("especialista"), pt) {
                    (true, false) => "Un especialista le dara seguimiento a su caso.",
                    (true, true) => "Um especialista fara o acompanhamento do seu caso.",
                    (false, false) => "Su disputa fue radicada.",
                    (false, true) => "Sua contestacao foi registrada.",
                };
                n.fetch_add(1, Ordering::SeqCst);
                let payload = json!({"output": out}).to_string();
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len());
            }
        });
        (url, served)
    }

    /// Native scorer for the prompt suite. `bound` = agent-core PR 50: a candidate (or control) prompt is exercised, so the native
    /// assertions pass. Without it every non-base draft that carries a prompt falls back to the template: the finding cases fail.
    fn native_prompt(bound: bool, break_guard: bool) -> impl Fn(&[Value]) -> Vec<(String, bool)> + 'static {
        let b = prompt_bundle();
        let ids = |k: &str| -> Vec<String> { b[k].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect() };
        let (f_ids, g_ids) = (ids("finding_case_ids"), ids("guard_case_ids"));
        move |changes| {
            let has_prompt = changes.iter().any(|c| c["kind"] == "prompt");
            let mut out: Vec<(String, bool)> = f_ids.iter().map(|c| (c.clone(), bound || !has_prompt)).collect();
            out.extend(g_ids.iter().enumerate().map(|(i, c)| (c.clone(), !(break_guard && has_prompt && i == 0))));
            out
        }
    }

    fn base_prompt() -> reasoning::catalog::Artifact {
        Catalog::bundled().get("prompt:p/resumen_radicado").unwrap().clone()
    }

    fn run_prompt(core: &Core, sc: &Fixed, changes: Vec<Value>) -> registry_writer::proof::Proof {
        let mem = MemoryStore::new();
        let w = Writer::new(cfg(Via::RegistryApi), core, &mem);
        let (fi, comp) = (finding(), prompt_compiled(changes));
        let sub = registry_writer::Submission::new(&fi, &comp);
        let base = base_prompt();
        let mut inp = input(&sub, &fi, &comp);
        inp.base_artifact = Some(&base);
        prove(&w, sc, &MemoryProofStore::default(), &opts(), &inp)
    }

    fn evaluated_prompt_texts(core: &Core) -> Vec<(String, Option<String>)> {
        core.log
            .borrow()
            .iter()
            .filter(|l| l.method == "PUT")
            .map(|l| {
                let ch = l.body.as_ref().unwrap()["changes"].as_array().unwrap().clone();
                let prompt = ch.iter().find(|c| c["kind"] == "prompt").map(|c| c["content"]["locales"]["es"].as_str().unwrap().to_string());
                (ch.iter().map(|c| c["kind"].as_str().unwrap()).collect::<Vec<_>>().join("+"), prompt)
            })
            .collect()
    }

    #[test]
    fn with_pr50_a_followup_prompt_is_proven_natively_and_by_the_probe_and_announced() {
        let (url, served) = gateway();
        let core = Core::new(native_prompt(true, false));
        let sc = scripts_for(prompt_bundle(), vec![("PULSO_PROBE_GATEWAY".into(), url)]);
        let p = run_prompt(&core, &sc, candidate("resumen_radicado_attempt2"));
        assert_eq!((p.announce, p.outcome.as_str()), (true, "announced"), "{}", p.story["reason"]);
        assert_eq!(p.story["native_binding"]["state"], "candidate_bound");
        assert_eq!(p.story["base"]["failed_cases"].as_array().unwrap().len(), 6, "the base fails by the wording probe only (native passes)");
        assert_eq!(p.story["base"]["native_verdict"], "pass");
        assert_eq!(p.story["attempts"][0]["verdict"], "pass");
        assert!(p.story["coverage"]["native"].as_array().unwrap().iter().any(|c| c == "response_from_model_path"));
        assert_eq!(served.load(Ordering::SeqCst), 12, "3 samples x (base es,pt + candidate es,pt); the control reuses the base text");
        // three evaluation drafts: base (suite), control (prompt with the BASE text and the candidate version, suite), candidate
        let drafts = evaluated_prompt_texts(&core);
        assert_eq!(drafts.iter().map(|d| d.0.as_str()).collect::<Vec<_>>(), ["eval_suite", "prompt+eval_suite", "prompt+eval_suite"]);
        assert_eq!(drafts[1].1.as_deref(), Some(base_prompt().locales["es"].as_str()), "the control is a text-identical bump");
        assert!(drafts[2].1.as_deref().unwrap().contains("especialista"));
        let put_control = core.log.borrow().iter().filter(|l| l.method == "PUT").nth(1).unwrap().body.clone().unwrap();
        assert_eq!(put_control["changes"][0]["content"]["version"], "1.0.1");
        assert_eq!(p.eval_proposals.len(), 3);
        let text = p.dossier["es"]["sections"]["coverage"].as_str().unwrap();
        assert!(text.contains("respuesta generada por el modelo") && text.contains("3 muestras reales") && text.contains("NO medido"), "{text}");
        assert!(p.dossier["pt"]["sections"]["coverage"].as_str().unwrap().contains("NÃO medido"));
        assert_eq!(p.record()["native_binding"]["state"], "candidate_bound");
    }

    #[test]
    fn with_pr50_a_text_identical_prompt_is_not_fixed_and_not_announced() {
        let (url, _) = gateway();
        let core = Core::new(native_prompt(true, false));
        let sc = scripts_for(prompt_bundle(), vec![("PULSO_PROBE_GATEWAY".into(), url)]);
        let p = run_prompt(&core, &sc, candidate("resumen_radicado_noop"));
        assert_eq!((p.announce, p.outcome.as_str()), (false, "not_announced:not_fixed"), "{}", p.story["reason"]);
        assert_eq!(p.story["native_binding"]["state"], "candidate_bound");
        assert_eq!(p.story["attempts"][0]["native_verdict"], "pass", "the native assertions cannot tell a no-op: only the probe does");
        assert_eq!(p.story["attempts"][0]["failed_cases"].as_array().unwrap().len(), 6);
        assert!(p.suite.is_none() && p.extra_changes.is_empty());
        assert!(p.dossier["es"]["description"].as_str().unwrap().contains("NO SE ANUNCIA"));
    }

    #[test]
    fn without_pr50_the_engine_detects_that_evaluate_ignored_the_candidate_prompt_labels_it_and_does_not_announce() {
        let (url, _) = gateway();
        let core = Core::new(native_prompt(false, false));
        let sc = scripts_for(prompt_bundle(), vec![("PULSO_PROBE_GATEWAY".into(), url)]);
        let p = run_prompt(&core, &sc, candidate("resumen_radicado_attempt2"));
        assert_eq!((p.announce, p.outcome.as_str()), (false, "not_announced:native_not_candidate_bound"), "{}", p.story["reason"]);
        assert_eq!(p.story["native_binding"]["state"], "native_not_candidate_bound");
        assert_eq!(p.story["probe_only_proven"], true, "the harness probe alone says it improves: labelled, not enough");
        assert_eq!(p.story["attempts"][0]["verdict"], "probe_only_pass");
        assert_eq!(p.story["attempts"][0]["native_verdict"], "fail", "agent-core failed the candidate natively: the template fallback");
        assert!(p.story["attempts"][0]["per_case"].as_object().unwrap().values().any(|c| c["source"] == "native_ignored"));
        assert!(p.suite.is_none() && p.extra_changes.is_empty());
        let d = &p.dossier["es"];
        assert!(d["sections"]["result"].as_str().unwrap().contains("SIN VÍNCULO NATIVO"), "{}", d["sections"]["result"]);
        assert!(d["sections"]["coverage"].as_str().unwrap().contains("sin vínculo al candidato"));
        assert!(!d["sections"]["coverage"].as_str().unwrap().contains("respuesta generada por el modelo"));
        assert_eq!(p.dossier["announce"], false);
    }

    #[test]
    fn without_pr50_a_text_identical_prompt_is_not_fixed_by_the_probe() {
        let (url, _) = gateway();
        let core = Core::new(native_prompt(false, false));
        let sc = scripts_for(prompt_bundle(), vec![("PULSO_PROBE_GATEWAY".into(), url)]);
        let p = run_prompt(&core, &sc, candidate("resumen_radicado_noop"));
        assert_eq!(p.outcome, "not_announced:not_fixed", "{}", p.story["reason"]);
        assert_eq!(p.story["native_binding"]["state"], "native_not_candidate_bound");
    }

    #[test]
    fn a_prompt_that_breaks_a_guard_is_guard_regressed_even_when_the_probe_passes() {
        let (url, _) = gateway();
        let core = Core::new(native_prompt(true, true));
        let sc = scripts_for(prompt_bundle(), vec![("PULSO_PROBE_GATEWAY".into(), url)]);
        let p = run_prompt(&core, &sc, candidate("resumen_radicado_attempt2"));
        assert_eq!(p.outcome, "not_announced:guard_regressed", "{}", p.story["reason"]);
    }

    #[test]
    fn an_unreachable_gateway_means_not_measured_never_announced() {
        let core = Core::new(native_prompt(true, false));
        let sc = scripts_for(prompt_bundle(), vec![("PULSO_PROBE_GATEWAY".into(), "http://127.0.0.1:1".into())]);
        let p = run_prompt(&core, &sc, candidate("resumen_radicado_attempt2"));
        assert!(!p.announce);
        let reason = p.story["base"]["per_case"].as_object().unwrap().values().find_map(|c| c["reason"].as_str().filter(|r| r.contains("not measured"))).map(str::to_string);
        assert!(reason.is_some(), "{}", p.story["base"]["per_case"]);
    }

    // ---------------------------------------------------------------------------------------------- new agent
    fn tecnico_compiled() -> Compiled {
        let f = finding();
        let row = map_finding(&f).unwrap();
        let o = Opportunity {
            id: "h_1".into(),
            target_ref: "new_agent:consultas".into(),
            mechanism_class: "uncovered_topic".into(),
            hypothesis: "h".into(),
            claimed_rate: 0.9,
            falsifiers: vec!["f".into()],
            alternatives: vec![Alt { kind: "do_nothing".into(), why_not: "x".into() }, Alt { kind: "human_owned".into(), why_not: "y".into() }],
        };
        let proposal = json!({"kind": "new_agent", "target_ref": "new_agent:consultas", "agent_id": "soporte-tecnico", "rationale": "A narrow intake for the uncovered topic.", "expected_direction": "decrease",
            "routing": {"summary_es": "Recibe problemas t\u{e9}cnicos de la aplicaci\u{f3}n y los pasa a una persona.", "summary_pt": "Recebe problemas t\u{e9}cnicos do aplicativo e os encaminha a uma pessoa.",
                        "examples_es": ["la app se cierra sola", "no puedo entrar a la aplicaci\u{f3}n"], "examples_pt": ["o aplicativo fecha sozinho", "n\u{e3}o consigo entrar no aplicativo"]},
            "intake": {"ask_es": "Cu\u{e9}ntame qu\u{e9} problema tienes con la aplicaci\u{f3}n.", "ask_pt": "Conte qual problema voc\u{ea} tem com o aplicativo.",
                       "notice_es": "Gracias, una persona del equipo te contactar\u{e1}.", "notice_pt": "Obrigado, uma pessoa da equipe vai falar com voc\u{ea}."},
            "alternatives": [{"kind": "do_nothing", "why_not": "x"}, {"kind": "human_owned", "why_not": "y"}], "uncertainty": "Where is known, why is not."});
        compile(&Catalog::bundled(), &f, &row, &o, &proposal).expect("the scripted new-agent proposal compiles")
    }

    fn native_agent(break_guard: bool) -> impl Fn(&[Value]) -> Vec<(String, bool)> + 'static {
        let b = agent_bundle();
        let ids = |k: &str| -> Vec<String> { b[k].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect() };
        let (f_ids, g_ids) = (ids("finding_case_ids"), ids("guard_case_ids"));
        move |_| {
            let mut out: Vec<(String, bool)> = f_ids.iter().map(|c| (c.clone(), true)).collect();
            out.extend(g_ids.iter().enumerate().map(|(i, c)| (c.clone(), !(break_guard && i == 0))));
            out
        }
    }

    fn run_agent(core: &Core, sc: &Fixed) -> (registry_writer::proof::Proof, Compiled) {
        let mem = MemoryStore::new();
        let w = Writer::new(cfg(Via::RegistryApi), core, &mem);
        let (fi, comp) = (finding(), tecnico_compiled());
        let sub = registry_writer::Submission::new(&fi, &comp);
        let inp = input(&sub, &fi, &comp);
        (prove(&w, sc, &MemoryProofStore::default(), &opts(), &inp), comp)
    }

    fn put_kinds(core: &Core) -> Vec<Vec<String>> {
        core.log.borrow().iter().filter(|l| l.method == "PUT").map(|l| l.body.as_ref().unwrap()["changes"].as_array().unwrap().iter().map(|c| c["kind"].as_str().unwrap().to_string()).collect()).collect()
    }

    #[test]
    fn a_new_agent_is_proven_on_itself_with_its_closure_and_announced_without_the_release_settings() {
        let core = Core::new(native_agent(false));
        let sc = scripts_for(agent_bundle(), vec![]);
        let (p, comp) = run_agent(&core, &sc);
        assert_eq!((p.announce, p.outcome.as_str()), (true, "announced"), "{}", p.story["reason"]);
        assert_eq!(sc.asked_agent.borrow().as_deref(), Some("soporte-tecnico"), "the suite is built for the new agent");
        // the base does not exist: nothing is evaluated for it, only the candidate draft is opened
        assert_eq!(p.story["base"]["verdict"], "absent");
        assert_eq!(p.eval_proposals.len(), 1);
        assert_eq!(core.count("POST", "/evaluate"), 1);
        let creates: Vec<Value> = core.log.borrow().iter().filter(|l| l.method == "POST" && l.path == "/v1/registry/proposals").map(|l| l.body.clone().unwrap()).collect();
        assert_eq!((creates[0]["agent_id"].as_str(), creates[0]["origin"].as_str()), (Some("soporte-tecnico"), Some("manual")));
        // evaluation draft: proposal + closure copies + evaluation-only ruleset and release settings + the suite
        let kinds = &put_kinds(&core)[0];
        for k in ["agent", "flow", "template", "decision_model", "language_detection", "injection_ruleset", "release_settings", "eval_suite"] {
            assert!(kinds.iter().any(|x| x == k), "{kinds:?} lacks {k}");
        }
        assert!(kinds.iter().filter(|k| *k == "template").count() >= 2 + 7, "2 own templates + the donor's template slots: {kinds:?}");
        // the finding cases fail on the base by absence (not measured), guards are n/a there
        assert_eq!(p.story["base"]["per_case"].as_object().unwrap().values().filter(|c| c["source"] == "absent_on_base").count(), 9);
        assert_eq!(p.story["base"]["failed_cases"].as_array().unwrap().len(), 6);
        assert_eq!(p.story["suite_is_regression_suite"], true);
        // delivered: the proposal + the closure copies + the suite; never the release settings nor the ruleset
        let sub = announce_submission(&finding(), &comp, &p);
        let dk: Vec<&str> = sub.changes.iter().map(|c| c["kind"].as_str().unwrap()).collect();
        assert_eq!(&dk[..4], ["agent", "flow", "template", "template"]);
        assert!(dk.contains(&"decision_model") && dk.contains(&"language_detection") && dk.last() == Some(&"eval_suite"));
        assert!(!dk.contains(&"release_settings") && !dk.contains(&"injection_ruleset"), "{dk:?}");
        // BLD1(a): what the value loop delivers carries the donor's entities, so Core never answers REG-PIN for the new agent
        let ids: Vec<&str> = sub.changes.iter().filter_map(|c| c["content"]["id"].as_str()).collect();
        assert!(ids.contains(&"understand-turno"), "the donor decision model travels: {ids:?}");
        assert!(sub.changes.iter().filter(|c| c["kind"] == "template" && c["docs"]["description"].as_str().is_some_and(|d| d.contains("closure copy"))).count() >= 7, "the donor templates travel: {ids:?}");
        assert_eq!(sub.agent_id, "soporte-tecnico");
        assert_eq!(sub.changes[0]["docs"]["changelog"], p.dossier["es"]["changelog"], "the proposal's own docs are the dossier's");
        assert_eq!(sub.changes[4]["docs"]["description"].as_str().map(|d| d.contains("closure copy")), Some(true), "closure copies keep their own docs");
        // honest dossier
        let cov = p.dossier["es"]["sections"]["coverage"].as_str().unwrap();
        for needle in ["ruteo de recepción al agente nuevo", "robo de tráfico", "SUPUESTO", "ajustes de release del donante", "NO medido"] {
            assert!(cov.contains(needle), "{needle}: {cov}");
        }
        assert!(p.dossier["es"]["sections"]["unchanged"].as_str().unwrap().contains("Recepción no cambia"));
        assert!(p.dossier["es"]["sections"]["result"].as_str().unwrap().contains("por ausencia"));
        for l in core.log.borrow().iter() {
            assert!(allowed(&l.method, &l.path), "{} {}", l.method, l.path);
        }
    }

    #[test]
    fn a_new_agent_whose_guard_fails_natively_is_not_announced() {
        let core = Core::new(native_agent(true));
        let (p, _) = run_agent(&core, &scripts_for(agent_bundle(), vec![]));
        assert_eq!((p.announce, p.outcome.as_str()), (false, "not_announced:guard_regressed"), "{}", p.story["reason"]);
        assert!(p.extra_changes.is_empty() && p.suite.is_none());
    }

    #[test]
    fn a_closure_that_cannot_be_read_is_an_internal_suite_error_and_nothing_is_evaluated() {
        let core = Core::new(native_agent(false));
        core.missing.borrow_mut().push("understand-turno".into());
        let (p, _) = run_agent(&core, &scripts_for(agent_bundle(), vec![]));
        assert_eq!((p.announce, p.outcome.as_str()), (false, "not_announced:suite_error"));
        assert!(p.story["reason"].as_str().unwrap().contains("understand-turno"), "{}", p.story["reason"]);
        assert_eq!(core.count("POST", "/evaluate"), 0);
    }

    #[test]
    fn a_script_set_without_new_agent_support_refuses_on_purpose() {
        struct Old;
        impl Scripts for Old {
            fn build_suite(&self, _: &Value, _: &str) -> Result<Value, SuiteError> {
                Ok(bundle())
            }
            fn judge(&self, _: &Value) -> Result<Value, String> {
                Err("not reached".into())
            }
        }
        let core = Core::new(native_agent(false));
        let mem = MemoryStore::new();
        let w = Writer::new(cfg(Via::RegistryApi), &core, &mem);
        let (fi, comp) = (finding(), tecnico_compiled());
        let sub = registry_writer::Submission::new(&fi, &comp);
        let p = prove(&w, &Old, &MemoryProofStore::default(), &opts(), &input(&sub, &fi, &comp));
        assert_eq!((p.announce, p.outcome.as_str()), (false, "not_announced:suite_refused"));
        assert!(core.log.borrow().is_empty());
    }
}
