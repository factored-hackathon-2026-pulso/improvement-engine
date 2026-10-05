//! B3: the value loop of `pulso run`. A `trigger:*` job (admitted by the automation endpoint into the real job store) runs the cells
//! sensor, the reasoning roles (scripted ports, labelled) and the registry writer (scripted wire) and records one outcome per finding
//! in the job store; per-finding idempotent; the engine never approves, publishes or promotes.
use core_client::authorizer::Jws;
use debug_api::Store;
use engine::models::ModelError;
use pg::repo::{JobRepository, MemRepo};
use pulso::config::RunConfig;
use pulso::run::engine_job::EngineRunner;
use pulso::run::supervisor::StopToken;
use pulso::run::tasks::{JobCtx, JobRunner};
use pulso::run::value_loop::{PortsFactory, ValueLoop};
use reasoning::finding::{Finding, Source};
use reasoning::pipeline::Ports;
use reasoning::testkit::{FnPort, synthetic_cells_ndjson};
use registry_writer::{Environment, Reply, Request, Transport, TransportError, Via};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const RUNNER: &str = env!("CARGO_BIN_EXE_pulso-synth-runner");
const T: &str = "tenant-local";
const KEY1: &str = "sha256:0f3a9c51d1b7e2a4c6b8d0e2f4a6c8e0a2b4d6f8091a2b3c4d5e6f708192a3b4";
const KEY2: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const TOKEN: &str = "fake.builder.token";

type Ans = Box<dyn Fn(&engine::models::ModelRequest) -> Result<Value, ModelError>>;

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pulso-vl-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("runs")).unwrap();
    d
}

/// A registry that behaves: entities are not served (fixture baseline), proposals are created, drafts stored and read back.
#[derive(Default)]
struct Wire {
    log: Mutex<Vec<(String, String)>>,
    changes: Mutex<usize>,
    next: Mutex<u32>,
}

impl Wire {
    fn count(&self, method: &str, path: &str) -> usize {
        self.log.lock().unwrap().iter().filter(|(m, p)| m == method && p == path).count()
    }
    fn paths(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().map(|(m, p)| format!("{m} {p}")).collect()
    }
}

impl Transport for Wire {
    fn send(&self, req: &Request) -> Result<Reply, TransportError> {
        assert_eq!(req.bearer.reveal(), TOKEN);
        self.log.lock().unwrap().push((req.method.into(), req.path.clone()));
        let (m, p) = (req.method, req.path.as_str());
        let r = |status, body| Ok(Reply { status, body });
        if m == "GET" && p.starts_with("/v1/registry/entities/") {
            return r(404, json!({"code": "not_found"}));
        }
        if m == "GET" && p.starts_with("/v1/registry/proposals?") {
            return r(200, json!({"items": [], "total": 0}));
        }
        if m == "POST" && p == "/v1/registry/proposals" {
            let mut n = self.next.lock().unwrap();
            *n += 1;
            return r(201, json!({"proposal_id": format!("prp_{n}"), "rev": 0, "state": "draft", "agent_id": "x"}));
        }
        if m == "PUT" && p.ends_with("/draft") {
            *self.changes.lock().unwrap() = req.body.as_ref().unwrap()["changes"].as_array().unwrap().len();
            return r(200, json!({"rev": 1}));
        }
        if m == "POST" && p.ends_with("/validate") {
            return r(200, json!({"violations": [], "candidate_hash": "h", "auto_bumped": []}));
        }
        if m == "GET" && p.starts_with("/v1/registry/proposals/") {
            let n = *self.changes.lock().unwrap();
            return r(200, json!({"proposal": {"proposal_id": "prp_1", "rev": 1, "state": "draft"}, "changes": vec![json!({}); n]}));
        }
        panic!("unexpected request {m} {p}");
    }
}

fn tecnico() -> Finding {
    let report: Value = serde_json::from_str(&steps::cells::run(&synthetic_cells_ndjson()).unwrap()).unwrap();
    let (f, _) = Finding::from_report(&report, Source::Synthetic).unwrap();
    f.into_iter().find(|x| x.dims.get("reason_category").map(String::as_str) == Some("Tecnico")).unwrap()
}

fn answers() -> (Ans, Ans, Ans) {
    let f = tecnico();
    let claimed = (f.discovery.numerator as f64 / f.discovery.denominator as f64 * 100.0).round() / 100.0;
    let scout: Ans = Box::new(move |_| {
        Ok(json!({"opportunity": {"id": "h_1", "target_ref": "new_agent:consultas", "mechanism_class": "uncovered_topic", "claimed_rate": claimed,
            "hypothesis": "The artifact has no wording for this situation, so the person has to ask again and the contact stays open.",
            "falsifiers": ["The rate is the same in contacts that did receive the new wording."],
            "alternatives": [{"kind": "do_nothing", "why_not": "The gap is replicated and material."}, {"kind": "human_owned", "why_not": "Thresholds and policies are not part of this change."}]}}))
    });
    let verifier: Ans = Box::new(|_| {
        let checks: Vec<Value> = ["recompute", "replication", "effect_size", "mechanism_fit", "dependency"].iter().map(|id| json!({"id": id, "result": "pass"})).collect();
        Ok(json!({"verdict": "supported", "checks": checks, "rationale": "The claim recomputes and replicates in the holdout."}))
    });
    let builder: Ans = Box::new(|_| {
        Ok(json!({"proposal": {"kind": "new_agent", "target_ref": "new_agent:consultas", "agent_id": "soporte-tecnico", "rationale": "A narrow intake for the uncovered topic.", "expected_direction": "decrease",
            "routing": {"summary_es": "Recibe problemas t\u{e9}cnicos de la aplicaci\u{f3}n y los pasa a una persona.", "summary_pt": "Recebe problemas t\u{e9}cnicos do aplicativo e os encaminha a uma pessoa.",
                        "examples_es": ["la app se cierra sola", "no puedo entrar a la aplicaci\u{f3}n"], "examples_pt": ["o aplicativo fecha sozinho", "n\u{e3}o consigo entrar no aplicativo"]},
            "intake": {"ask_es": "Cu\u{e9}ntame qu\u{e9} problema tienes con la aplicaci\u{f3}n.", "ask_pt": "Conte qual problema voc\u{ea} tem com o aplicativo.",
                       "notice_es": "Gracias, una persona del equipo te contactar\u{e1}.", "notice_pt": "Obrigado, uma pessoa da equipe vai falar com voc\u{ea}."},
            "alternatives": [{"kind": "do_nothing", "why_not": "The effect is replicated."}, {"kind": "other_target", "why_not": "No other artifact carries this wording."}], "uncertainty": "The data says where the problem is, not why."}}))
    });
    (scout, verifier, builder)
}

fn scripted_ports() -> PortsFactory {
    Arc::new(|| {
        let (s, v, b) = answers();
        Ok(Ports {
            scout: Rc::new(FnPort::scripted("scripted-scout", s)),
            verifier: Rc::new(FnPort::scripted("scripted-verifier", v)),
            builder: Rc::new(FnPort::scripted("scripted-builder", b)),
            builder_escalation: None,
        })
    })
}

fn value_loop(work: &Path, wire: Arc<dyn Transport + Send + Sync>) -> ValueLoop {
    let cells = work.join("cells.ndjson");
    std::fs::write(&cells, synthetic_cells_ndjson()).unwrap();
    ValueLoop {
        cells,
        source: Source::Synthetic,
        allow_derived: false,
        receipts: work.join("receipts.json"),
        via: Via::RegistryApi,
        environment: Environment::LocalStack,
        credential: "engine builder principal",
        registry_token: Jws::new(TOKEN.into()),
        run_token: None,
        transport: wire,
        ports: scripted_ports(),
        model_label: "scripted".into(),
        max_findings: None,
        proof: None,
        announcer: None,
        caps: Default::default(),
    }
}

fn config(work: &Path) -> RunConfig {
    let m: HashMap<String, String> = [("PULSO_STORAGE", "memory"), ("PULSO_DATA_MODE", "platform"), ("PULSO_SOURCE_ADAPTER", "product-sqlite"), ("PULSO_SOURCE_ID", "platform:sim"), ("PULSO_WORK_DIR", work.to_str().unwrap()), ("PULSO_SOURCE_SQLITE", "unused.db"), ("PULSO_LISTEN_ADDR", "127.0.0.1:0")]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    RunConfig::from_lookup(&|k| m.get(k).cloned()).unwrap()
}

fn runner(work: &Path, v: ValueLoop) -> (EngineRunner, Arc<Store>) {
    let store = Arc::new(Store::memory());
    (EngineRunner::new(&config(work), work, Path::new(RUNNER), store.clone()).unwrap().with_value_loop(Some(Arc::new(v))), store)
}

fn run_key(r: &EngineRunner, repo: &MemRepo, key: &str) -> (String, Result<(), String>) {
    let id = repo.admit_keyed(T, key).unwrap();
    let c = repo.claim_next(T, "w", 100, 60).unwrap().unwrap();
    assert_eq!(c.job, id);
    let stop = StopToken::new();
    let ctx = JobCtx { repo, tenant: T, worker: "w", stop: &stop, now: 100, lease_seconds: 60 };
    let res = r.run(&c, &ctx);
    (id, res)
}

#[test]
fn a_trigger_job_runs_the_loop_delivers_one_proposal_and_records_the_outcome_in_the_job_store() {
    let work = temp("trigger");
    let wire = Arc::new(Wire::default());
    let (r, store) = runner(&work, value_loop(&work, wire.clone()));
    let repo = MemRepo::new();
    let (job, res) = run_key(&r, &repo, &format!("trigger:{KEY1}"));
    res.unwrap();
    let summary: Value = serde_json::from_str(&repo.output(T, &job, 0).unwrap().expect("summary at step 0")).unwrap();
    let s = &summary["value_loop"]["summary"];
    assert_eq!((s["corroborated"].as_u64(), s["proposed"].as_u64(), s["delivered"].as_u64(), s["denied"].as_u64()), (Some(1), Some(1), Some(1), Some(0)), "{summary}");
    let rec: Value = serde_json::from_str(&repo.output(T, &job, 1).unwrap().expect("finding record at step 1")).unwrap();
    assert_eq!((rec["status"].as_str(), rec["delivery"]["status"].as_str(), rec["delivery"]["proposal_id"].as_str()), (Some("proposed"), Some("delivered"), Some("prp_1")));
    assert_eq!(rec["independence"]["level"], "other_family", "scripted ids differ and have no common vendor");
    assert_eq!(summary["value_loop"]["engine_never_approves_publishes_or_promotes"], true);
    assert!(!summary.to_string().contains(TOKEN));
    // honest labels: fixture baseline (the wire serves no entity), claude-standin sensor
    assert_eq!(summary["value_loop"]["baseline"]["live"], 0);
    assert!(summary["value_loop"]["sensor"].as_str().unwrap().contains("claude-standin"));
    // the writer only ever created, wrote a draft and validated
    for p in wire.paths() {
        assert!(!p.contains("approve") && !p.contains("publish") && !p.contains("promote") && !p.contains("freeze") && !p.contains("evaluate"), "{p}");
    }
    assert_eq!(wire.count("POST", "/v1/registry/proposals"), 1);
    // the console store shows the loop run
    let runs = store.runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(store.state(&runs[0]).unwrap()["run"]["state"], "completed");
}

#[test]
fn a_second_trigger_for_the_same_findings_opens_no_second_proposal() {
    let work = temp("idem");
    let wire = Arc::new(Wire::default());
    let (r, _) = runner(&work, value_loop(&work, wire.clone()));
    let repo = MemRepo::new();
    run_key(&r, &repo, &format!("trigger:{KEY1}")).1.unwrap();
    let (job2, res) = run_key(&r, &repo, &format!("trigger:{KEY2}"));
    res.unwrap();
    assert_eq!(wire.count("POST", "/v1/registry/proposals"), 1, "same evidence, same target, same key: the receipt is reused");
    let rec: Value = serde_json::from_str(&repo.output(T, &job2, 1).unwrap().unwrap()).unwrap();
    assert_eq!((rec["delivery"]["status"].as_str(), rec["delivery"]["replayed"].as_bool()), (Some("delivered"), Some(true)));
}

#[test]
fn a_replayed_job_reuses_its_finding_records_and_calls_no_model_again() {
    let work = temp("resume");
    let wire = Arc::new(Wire::default());
    let first = value_loop(&work, wire.clone()).run(&pulso::run::value_loop::NoPersist).unwrap();
    let held = first["findings"][0].to_string();
    struct Held(String);
    impl pulso::run::value_loop::Persist for Held {
        fn get(&self, s: u32) -> Option<String> {
            (s == 1).then(|| self.0.clone())
        }
        fn put(&self, _: u32, _: &str) -> Result<(), String> {
            Err("must not write: the record is held".into())
        }
    }
    let mut v2 = value_loop(&work, wire.clone());
    v2.ports = Arc::new(|| Err("no model may be called on a replay".into()));
    let out = v2.run(&Held(held)).unwrap();
    assert_eq!(out["findings"][0]["resumed_from_job_store"], true);
    assert_eq!(wire.count("POST", "/v1/registry/proposals"), 1);
}

#[test]
fn a_denied_delivery_is_an_outcome_with_a_closed_reason_not_a_failed_job() {
    struct Down;
    impl Transport for Down {
        fn send(&self, _: &Request) -> Result<Reply, TransportError> {
            Ok(Reply { status: 403, body: json!({"code": "forbidden", "detail": "free text must not travel"}) })
        }
    }
    let work = temp("denied");
    let (r, _) = runner(&work, value_loop(&work, Arc::new(Down)));
    let repo = MemRepo::new();
    let (job, res) = run_key(&r, &repo, &format!("trigger:{KEY1}"));
    res.unwrap();
    let rec: Value = serde_json::from_str(&repo.output(T, &job, 1).unwrap().unwrap()).unwrap();
    assert_eq!(rec["delivery"]["status"], "denied");
    assert!(rec["delivery"]["reason"].is_string());
    assert!(!rec.to_string().contains("free text"));
}

#[test]
fn a_trigger_job_without_a_configured_loop_is_recorded_as_skipped_not_dropped() {
    let work = temp("off");
    let r = EngineRunner::new(&config(&work), &work, Path::new(RUNNER), Arc::new(Store::memory())).unwrap();
    let repo = MemRepo::new();
    let (job, res) = run_key(&r, &repo, &format!("trigger:{KEY1}"));
    res.unwrap();
    let s: Value = serde_json::from_str(&repo.output(T, &job, 0).unwrap().unwrap()).unwrap();
    assert!(s["value_loop"]["skipped"].is_string(), "{s}");
}

#[test]
fn configuration_is_off_without_cells_and_refuses_a_half_configured_loop() {
    let get = |m: HashMap<&'static str, &'static str>| move |k: &str| m.get(k).map(|v| v.to_string());
    assert!(ValueLoop::from_lookup(&get(HashMap::new()), None).unwrap().is_none());
    let e = ValueLoop::from_lookup(&get([("PULSO_CELLS_NDJSON", "c.ndjson")].into()), None).err().unwrap();
    assert!(e.contains("PULSO_CELLS_SOURCE"), "{e}");
    let e = ValueLoop::from_lookup(&get([("PULSO_CELLS_NDJSON", "c.ndjson"), ("PULSO_CELLS_SOURCE", "bank")].into()), None).err().unwrap();
    assert!(e.contains("PULSO_REGISTRY_ADDR"), "{e}");
    let full = [("PULSO_CELLS_NDJSON", "c.ndjson"), ("PULSO_CELLS_SOURCE", "bank"), ("PULSO_REGISTRY_ADDR", "127.0.0.1:1"), ("PULSO_REGISTRY_TOKEN", "sekret")];
    let e = ValueLoop::from_lookup(&get(full.into()), Some(Path::new("."))).err().unwrap();
    assert!(e.contains("gateway") && !e.contains("sekret"), "the loop has no scripted model configuration: {e}");
}

fn http(addr: std::net::SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    let h: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: x\r\n{h}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    (raw.split(' ').nth(1).unwrap().parse().unwrap(), raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}

#[test]
fn the_trigger_endpoint_of_pulso_admits_into_the_real_job_store_and_the_runner_executes_it() {
    use pulso::health::{DbProbe, Health};
    use pulso::run::http::HttpTask;
    use pulso::run::log::Logger;
    use pulso::run::supervisor::Supervisor;
    struct Up;
    impl DbProbe for Up {
        fn ping(&self) -> Result<(), String> {
            Ok(())
        }
    }
    let work = temp("http");
    let wire = Arc::new(Wire::default());
    let (r, _) = runner(&work, value_loop(&work, wire.clone()));
    let repo = Arc::new(MemRepo::new());
    let health = Health::new(Arc::new(Up));
    let task = HttpTask::bind_with(&config(&work), health.clone(), Arc::new(Store::memory()), Some(repo.clone())).unwrap();
    let addr = task.local_addr();
    let sink: Box<dyn std::io::Write + Send> = Box::new(std::io::sink());
    let mut sup = Supervisor::new(health, Logger::new(sink), std::time::Duration::from_secs(2));
    sup.add(Box::new(task));
    let stop = sup.stop_token();
    let join = std::thread::spawn(move || sup.run());
    let (st, body) = http(addr, "GET", "/api/v1/auth/session", &[], "");
    assert_eq!(st, 200);
    let csrf = serde_json::from_str::<Value>(&body).unwrap()["csrf_token"].as_str().unwrap().to_string();
    let trig = json!({"schema": "pulso.trigger.v1", "trigger_key": KEY1, "kind": "explicit", "tenant": T, "mission": "m-quejas", "source": "agent-core", "config_digest": "sha256:abc123",
        "event": {"type": "run.now", "ref": "evt-0001", "at": "2026-10-04T10:00:00Z", "subject": {}}, "requested_at": "2026-10-04T10:00:01Z"})
    .to_string();
    let path = "/internal/v1/automation/triggers";
    assert_eq!(http(addr, "POST", path, &[("Idempotency-Key", KEY1)], &trig).0, 403, "no csrf: nothing admitted");
    let h = [("Idempotency-Key", KEY1), ("X-CSRF-Token", csrf.as_str())];
    let (st, body) = http(addr, "POST", path, &h, &trig);
    assert_eq!(st, 202, "{body}");
    let job = serde_json::from_str::<Value>(&body).unwrap()["job_id"].as_str().unwrap().to_string();
    assert_eq!(http(addr, "POST", path, &h, &trig).0, 202, "replay");
    // the job is in the real store under the trigger key; the worker's claim hands it to the runner
    let c = repo.claim_next(T, "w", 100, 60).unwrap().expect("one job admitted");
    assert_eq!(c.job, job);
    assert_eq!(repo.job_key(T, &job).unwrap().as_deref(), Some(format!("trigger:{KEY1}").as_str()));
    assert!(repo.claim_next(T, "w2", 100, 60).unwrap().is_none(), "the replay queued nothing");
    let st = StopToken::new();
    let ctx = JobCtx { repo: repo.as_ref(), tenant: T, worker: "w", stop: &st, now: 100, lease_seconds: 60 };
    r.run(&c, &ctx).unwrap();
    assert_eq!(wire.count("POST", "/v1/registry/proposals"), 1);
    stop.stop();
    let _ = join.join();
}

// ---------------------------------------------------------------------------------------------------- W11: evaluate before announce
mod w11 {
    use super::*;
    use pulso::run::value_loop::ProofConfig;
    use registry_writer::eval::EvalOptions;
    use registry_writer::proof::{Scripts, SuiteError};

    /// build_suite refuses (or answers a minimal bundle) and the judge answers a fixed story. The Python judge itself is tested in
    /// registry-writer/tests/proof.rs.
    struct Fake {
        refuse: bool,
        story_outcome: &'static str,
    }

    impl Scripts for Fake {
        fn build_suite_for(&self, f: &Value, t: &str, _new_agent: Option<&str>) -> Result<Value, SuiteError> {
            self.build_suite(f, t)
        }
        fn build_suite(&self, _: &Value, _: &str) -> Result<Value, SuiteError> {
            if self.refuse {
                return Err(SuiteError::Refused("k_below_minimum".into(), "thin".into()));
            }
            Ok(json!({"agent": "consultas", "suite": {"id": "reg-consultas-aa", "version": "1.0.0", "agent_id": "consultas", "thresholds": {}, "scenarios": [{"id": "c1"}, {"id": "guard-g1"}]},
                      "finding_case_ids": ["c1"], "guard_case_ids": ["guard-g1"]}))
        }
        fn judge(&self, input: &Value) -> Result<Value, String> {
            let proven = self.story_outcome == "regression_suite_proven";
            let attempts = input["attempts"].as_array().cloned().unwrap_or_default();
            let base_only = attempts.is_empty();
            Ok(json!({"schema": "reg1.verdict_story/1", "outcome": if base_only { "base_only" } else { self.story_outcome }, "reason": "scripted", "announce": proven && !base_only,
                      "suite_id": "reg-consultas-aa", "mechanism": "scripted", "suite_is_regression_suite": proven,
                      "base": {"label": "base", "verdict": "fail", "per_case": {"c1": {"passed": false}, "guard-g1": {"passed": true}}, "failed_cases": ["c1"], "guards_failed": [], "gate_items": [], "infra_retries": []},
                      "attempts": attempts.iter().map(|a| json!({"label": "candidate-1", "attempt": 1, "verdict": if proven { "pass" } else { "fail" }, "failed_cases": if proven { json!([]) } else { json!(["c1"]) }, "guards_failed": [], "gate_items": [{"metric": "scenario/c1", "phase": "gate", "passed": proven}], "proposal_id": a["run"]["proposal_id"]})).collect::<Vec<_>>(),
                      "gate_items": [{"metric": "scenario/c1", "phase": "gate", "passed": proven}], "story_text": {"es": "scripted", "pt": "scripted"}, "model_policy": {"judge": "none", "judge_rule": "deterministic checks only"}}))
        }
    }

    /// Registry double that also freezes and evaluates; remembers the origin of every proposal it created.
    #[derive(Default)]
    struct Core {
        origins: Mutex<Vec<String>>,
        paths: Mutex<Vec<String>>,
        n: Mutex<u32>,
        changes: Mutex<usize>,
    }

    impl Transport for Core {
        fn send(&self, req: &Request) -> Result<Reply, TransportError> {
            assert_eq!(req.bearer.reveal(), TOKEN);
            let (m, p) = (req.method, req.path.as_str());
            self.paths.lock().unwrap().push(format!("{m} {p}"));
            let r = |status, body| Ok(Reply { status, body });
            if m == "GET" && p.starts_with("/v1/registry/entities/") {
                // the donor closure of a new-agent proposal (templates, model, tools, language detection, ruleset) is readable
                let kind = p.trim_start_matches("/v1/registry/entities/").split('/').next().unwrap_or("");
                if ["template", "decision_model", "tool", "language_detection", "injection_ruleset"].contains(&kind) {
                    return r(200, json!({"content": {"id": "donor-copy", "version": "1.0.0"}}));
                }
                return r(404, json!({"code": "not_found"}));
            }
            if m == "GET" && p.starts_with("/v1/registry/proposals?") {
                return r(200, json!({"items": [], "total": 0}));
            }
            if m == "POST" && p == "/v1/registry/proposals" {
                self.origins.lock().unwrap().push(req.body.as_ref().unwrap()["origin"].as_str().unwrap().to_string());
                let mut n = self.n.lock().unwrap();
                *n += 1;
                return r(201, json!({"proposal_id": format!("prp_{n}"), "rev": 0, "state": "draft"}));
            }
            if m == "PUT" && p.ends_with("/draft") {
                *self.changes.lock().unwrap() = req.body.as_ref().unwrap()["changes"].as_array().unwrap().len();
                return r(200, json!({"rev": 1}));
            }
            if m == "POST" && p.ends_with("/validate") {
                return r(200, json!({"violations": [], "candidate_hash": "h"}));
            }
            if m == "POST" && p.ends_with("/freeze") {
                return r(200, json!({"candidate_hash": "h"}));
            }
            if m == "POST" && p.ends_with("/evaluate") {
                return r(200, json!({"verdict": "pass", "items": [], "runs": {"cand_on_new": {"scenarios": {"c1": true, "guard-g1": true}}}, "results": []}));
            }
            if m == "GET" && p.starts_with("/v1/registry/proposals/") {
                let n = *self.changes.lock().unwrap();
                return r(200, json!({"proposal": {"proposal_id": "prp_x", "rev": 1, "state": "draft"}, "changes": vec![json!({}); n]}));
            }
            panic!("unexpected request {m} {p}");
        }
    }

    fn with_proof(work: &Path, core: Arc<Core>, fake: Fake) -> ValueLoop {
        let mut v = value_loop(work, core.clone());
        v.proof = Some(ProofConfig {
            scripts: Arc::new(fake),
            eval_transport: core,
            opts: EvalOptions { infra_retries: 0, backoff: std::time::Duration::ZERO, sleep: |_| {} },
            proofs: work.join("w11-proofs.json"),
        });
        v
    }

    #[test]
    fn a_proven_finding_is_announced_as_auto_detect_after_two_manual_evaluation_drafts() {
        let work = temp("w11-proven");
        let core = Arc::new(Core::default());
        let out = with_proof(&work, core.clone(), Fake { refuse: false, story_outcome: "regression_suite_proven" }).run(&pulso::run::value_loop::NoPersist).unwrap();
        let rec = &out["findings"][0];
        assert_eq!(rec["outcome"], "announced", "{rec}");
        assert_eq!(rec["delivery"]["status"], "delivered");
        assert_eq!(rec["evaluation"]["verdict"], "regression_suite_proven");
        assert_eq!(rec["evaluation"]["dossier"]["announce"], true);
        assert_eq!(rec["evaluation"]["labels"]["calibration"], "uncalibrated");
        assert_eq!(*core.origins.lock().unwrap(), vec!["manual", "auto_detect"], "the base has no new agent (not evaluated): only the candidate evaluation draft; it never uses the auto_detect quota, the announced proposal does");
        assert_eq!(out["summary"]["announced"], 1);
        assert_eq!(out["evaluate_before_announce"], "on");
        let paths = core.paths.lock().unwrap().join("\n");
        assert!(!paths.contains("approve") && !paths.contains("publish") && !paths.contains("promote") && !paths.contains("reject"), "{paths}");
        assert!(!out.to_string().contains(TOKEN));
    }

    /// Platform double: answers a fixed status for every announce and remembers what it was sent.
    struct Platform {
        status: u16,
        sent: Mutex<Vec<(String, Value)>>,
    }
    impl Transport for Platform {
        fn send(&self, req: &Request) -> Result<Reply, TransportError> {
            assert_eq!(req.bearer.reveal(), "platform-throwaway");
            self.sent.lock().unwrap().push((format!("{} {}", req.method, req.path), req.body.clone().unwrap()));
            Ok(Reply { status: self.status, body: json!({"proposalId": req.body.as_ref().unwrap()["proposalId"]}) })
        }
    }
    fn announcing(work: &Path, core: Arc<Core>, fake: Fake, p: Arc<Platform>) -> ValueLoop {
        let mut v = with_proof(work, core, fake);
        let mut a = registry_writer::announce::Announcer::new(p, Jws::new("platform-throwaway".into()));
        a.sleep = |_| {};
        v.announcer = Some(a);
        v
    }

    #[test]
    fn an_announced_proposal_is_relayed_to_the_platform_after_agent_core_accepted_it() {
        let work = temp("ann1-ok");
        let p = Arc::new(Platform { status: 201, sent: Mutex::new(vec![]) });
        let out = announcing(&work, Arc::new(Core::default()), Fake { refuse: false, story_outcome: "regression_suite_proven" }, p.clone()).run(&pulso::run::value_loop::NoPersist).unwrap();
        let rec = &out["findings"][0];
        assert_eq!((rec["outcome"].as_str(), rec["delivery"]["status"].as_str(), rec["platform_announce"].as_str()), (Some("announced"), Some("delivered"), Some("platform_announced")), "{rec}");
        let sent = p.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "POST /api/v1/internal/builder/proposals/announce");
        assert_eq!(sent[0].1["proposalId"], rec["delivery"]["proposal_id"]);
        assert!(!out.to_string().contains("platform-throwaway"));
    }

    #[test]
    fn a_failing_platform_never_fails_the_agent_core_delivery() {
        let work = temp("ann1-down");
        let p = Arc::new(Platform { status: 503, sent: Mutex::new(vec![]) });
        let out = announcing(&work, Arc::new(Core::default()), Fake { refuse: false, story_outcome: "regression_suite_proven" }, p.clone()).run(&pulso::run::value_loop::NoPersist).unwrap();
        let rec = &out["findings"][0];
        assert_eq!((rec["outcome"].as_str(), rec["delivery"]["status"].as_str(), rec["platform_announce"].as_str()), (Some("announced"), Some("delivered"), Some("platform_announce_failed:unavailable")), "{rec}");
        assert_eq!(p.sent.lock().unwrap().len(), 3, "bounded retries");
    }

    #[test]
    fn a_not_announced_finding_never_calls_the_platform() {
        let work = temp("ann1-none");
        let p = Arc::new(Platform { status: 201, sent: Mutex::new(vec![]) });
        let out = announcing(&work, Arc::new(Core::default()), Fake { refuse: false, story_outcome: "not_fixed" }, p.clone()).run(&pulso::run::value_loop::NoPersist).unwrap();
        assert!(out["findings"][0]["platform_announce"].is_null());
        assert!(p.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn a_non_proven_finding_is_kept_as_an_internal_outcome_with_its_dossier_and_nothing_is_announced() {
        let work = temp("w11-notfixed");
        let core = Arc::new(Core::default());
        let out = with_proof(&work, core.clone(), Fake { refuse: false, story_outcome: "not_fixed" }).run(&pulso::run::value_loop::NoPersist).unwrap();
        let rec = &out["findings"][0];
        assert_eq!(rec["outcome"], "not_announced:not_fixed", "{rec}");
        assert!(rec["delivery"].is_null());
        assert_eq!(rec["evaluation"]["dossier"]["announce"], false);
        assert!(rec["evaluation"]["dossier"]["es"]["description"].as_str().unwrap().contains("NO SE ANUNCIA"));
        assert_eq!(*core.origins.lock().unwrap(), vec!["manual"], "no auto_detect proposal exists (new agent: only the candidate was evaluated)");
        assert_eq!((out["summary"]["announced"].as_u64(), out["summary"]["not_announced"].as_u64(), out["summary"]["delivered"].as_u64()), (Some(0), Some(1), Some(0)));
    }

    #[test]
    fn a_refused_suite_is_not_announced_and_touches_no_registry() {
        let work = temp("w11-refused");
        let core = Arc::new(Core::default());
        let out = with_proof(&work, core.clone(), Fake { refuse: true, story_outcome: "" }).run(&pulso::run::value_loop::NoPersist).unwrap();
        assert_eq!(out["findings"][0]["outcome"], "not_announced:suite_refused");
        assert!(core.origins.lock().unwrap().is_empty());
    }

    #[test]
    fn the_proof_is_off_when_not_configured_and_the_old_delivery_is_unchanged() {
        let work = temp("w11-off");
        let wire = Arc::new(Wire::default());
        let out = value_loop(&work, wire.clone()).run(&pulso::run::value_loop::NoPersist).unwrap();
        assert_eq!(out["evaluate_before_announce"], "off");
        assert!(out["findings"][0]["outcome"].is_null() && out["findings"][0]["delivery"]["status"] == "delivered");
    }

    #[test]
    fn from_lookup_turns_the_proof_on_for_the_live_path_unless_it_is_switched_off() {
        let work = temp("w11-cfg");
        let w = work.to_str().unwrap().to_string();
        let mk = |extra: &[(&str, &str)]| {
            let mut m: HashMap<String, String> = [("PULSO_CELLS_NDJSON", "c.ndjson"), ("PULSO_CELLS_SOURCE", "synthetic"), ("PULSO_REGISTRY_ADDR", "127.0.0.1:1"), ("PULSO_REGISTRY_TOKEN", "t"), ("PULSO_LLM_GATEWAY", "enabled"), ("PULSO_LLM_GATEWAY_ADDR", "127.0.0.1:1"), ("PULSO_LLM_GATEWAY_KEY", "k")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            m.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
            ValueLoop::from_lookup(&move |k| m.get(k).cloned(), Some(Path::new(&w)))
        };
        assert!(mk(&[]).unwrap().unwrap().proof.is_some(), "live path default: ON");
        assert!(mk(&[("PULSO_EVAL_BEFORE_ANNOUNCE", "off")]).unwrap().unwrap().proof.is_none());
        assert!(mk(&[("PULSO_REGISTRY_VIA", "run")]).unwrap().unwrap().proof.is_none(), "a builder-run draft cannot be proven here");
        assert!(mk(&[("PULSO_REGISTRY_VIA", "run"), ("PULSO_EVAL_BEFORE_ANNOUNCE", "on")]).err().unwrap().contains("PULSO_REGISTRY_VIA=api"));
        assert!(mk(&[("PULSO_EVAL_BEFORE_ANNOUNCE", "maybe")]).err().unwrap().contains("on|off"));
    }
    // ------------------------------------------------------------------------------------------------------------ MAP1: candidates
    /// build_suite answers for every target; the judge proves ONLY the target named in `proven`.
    struct ByTarget {
        proven: &'static str,
        last: Mutex<String>,
        built: Mutex<Vec<String>>,
    }
    impl Scripts for ByTarget {
        fn build_suite_for(&self, f: &Value, t: &str, _new_agent: Option<&str>) -> Result<Value, SuiteError> {
            self.build_suite(f, t)
        }
        fn build_suite(&self, _: &Value, t: &str) -> Result<Value, SuiteError> {
            *self.last.lock().unwrap() = t.to_string();
            self.built.lock().unwrap().push(t.to_string());
            Ok(json!({"agent": "consultas", "suite": {"id": "reg-consultas-aa", "version": "1.0.0", "agent_id": "consultas", "thresholds": {}, "scenarios": [{"id": "c1"}, {"id": "guard-g1"}]},
                      "finding_case_ids": ["c1"], "guard_case_ids": ["guard-g1"]}))
        }
        fn judge(&self, input: &Value) -> Result<Value, String> {
            let proven = *self.last.lock().unwrap() == self.proven;
            let attempts = input["attempts"].as_array().cloned().unwrap_or_default();
            let base_only = attempts.is_empty();
            Ok(json!({"schema": "reg1.verdict_story/1", "outcome": if base_only { "base_only" } else if proven { "regression_suite_proven" } else { "not_fixed" }, "reason": "scripted", "announce": proven && !base_only,
                      "suite_id": "reg-consultas-aa", "mechanism": "scripted", "suite_is_regression_suite": proven,
                      "base": {"label": "base", "verdict": "fail", "per_case": {"c1": {"passed": false}, "guard-g1": {"passed": true}}, "failed_cases": ["c1"], "guards_failed": [], "gate_items": [], "infra_retries": []},
                      "attempts": attempts.iter().map(|a| json!({"label": "candidate-1", "attempt": 1, "verdict": if proven { "pass" } else { "fail" }, "failed_cases": if proven { json!([]) } else { json!(["c1"]) }, "guards_failed": [], "gate_items": [{"metric": "scenario/c1", "phase": "gate", "passed": proven}], "proposal_id": a["run"]["proposal_id"]})).collect::<Vec<_>>(),
                      "gate_items": [{"metric": "scenario/c1", "phase": "gate", "passed": proven}], "story_text": {"es": "scripted", "pt": "scripted"}, "model_policy": {"judge": "none", "judge_rule": "deterministic checks only"}}))
        }
    }

    /// Queja/Phone planted at 55 percent against 20 percent everywhere else, in both halves.
    fn queja_cells() -> String {
        let mut rows = vec![];
        for r in ["Queja", "Tecnico", "Comercial", "Retencion", "Transaccional", "Producto"] {
            for c in ["Phone", "Chat"] {
                for (half, den) in [("discovery", 6000i64), ("holdout", 4000i64)] {
                    let permille = if r == "Queja" && c == "Phone" { 550 } else { 200 };
                    rows.push(format!("{{\"metric\":\"M1\",\"dims\":{{\"reason_category\":\"{r}\",\"channel\":\"{c}\"}},\"half\":\"{half}\",\"numerator\":{},\"denominator\":{den}}}", den * permille / 1000));
                }
            }
        }
        rows.join("\n")
    }

    fn candidate_ports() -> PortsFactory {
        Arc::new(|| {
            let scout: Ans = Box::new(|req| {
                let t = &req.payload["inputs"]["allowed_targets"][0];
                assert_eq!(req.payload["inputs"]["allowed_targets"].as_array().unwrap().len(), 1, "one candidate per attempt");
                Ok(json!({"opportunity": {"id": "h_1", "target_ref": t["target_ref"], "mechanism_class": t["mechanisms"][0], "claimed_rate": 0.55,
                    "hypothesis": "The artifact has no wording for this situation, so the person has to ask again and the contact stays open.",
                    "falsifiers": ["The rate is the same in contacts that did receive the new wording."],
                    "alternatives": [{"kind": "do_nothing", "why_not": "The gap is replicated and material."}, {"kind": "human_owned", "why_not": "Thresholds and policies are not part of this change."}]}}))
            });
            let (_, verifier, _) = answers();
            let builder: Ans = Box::new(|req| {
                let alts = json!([{"kind": "do_nothing", "why_not": "The effect is replicated."}, {"kind": "other_target", "why_not": "No other artifact carries this wording."}]);
                let menu = reasoning::testkit::menu_of(req);
                let anchor = |loc: &str, needle: &str| menu.iter().find(|(id, t)| id.starts_with(&format!("{loc}.")) && t.contains(needle)).unwrap_or_else(|| panic!("no {loc} anchor with {needle}")).0.clone();
                match req.payload["inputs"]["target_ref"].as_str().unwrap() {
                    "template:t/estado_pqr" => Ok(json!({"proposal": {"kind": "patch", "rationale": "Say what the status is.", "alternatives": alts, "uncertainty": "Association only.",
                        "patches": [{"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": "Ya consult\u{e9} tu PQR y su estado actual es {{ facts.pqr.value.status }}."},
                                    {"locale": "pt", "anchor_id": "pt.a1", "op": "replace", "replacement": "J\u{e1} consultei sua solicita\u{e7}\u{e3}o e o estado atual \u{e9} {{ facts.pqr.value.status }}."}]}})),
                    "prompt:p/resumen_radicado" => Ok(json!({"proposal": {"kind": "patch", "rationale": "Name who follows up.", "alternatives": alts, "uncertainty": "Association only.",
                        "patches": [{"locale": "es", "anchor_id": anchor("es", "Redacta"), "op": "insert_after", "replacement": "Di que una persona del equipo har\u{e1} el seguimiento del caso."},
                                    {"locale": "pt", "anchor_id": anchor("pt", "Redija"), "op": "insert_after", "replacement": "Diga que uma pessoa da equipe far\u{e1} o acompanhamento do caso."}]}})),
                    other => panic!("unexpected target {other}"),
                }
            });
            Ok(Ports { scout: Rc::new(FnPort::scripted("scripted-scout", scout)), verifier: Rc::new(FnPort::scripted("scripted-verifier", verifier)), builder: Rc::new(FnPort::scripted("scripted-builder", builder)), builder_escalation: None })
        })
    }

    /// The Core double, except that templates and prompts are served with the texts of the bundled baseline (the live base of a patch).
    struct PatchCore(Arc<Core>);
    impl Transport for PatchCore {
        fn send(&self, req: &Request) -> Result<Reply, TransportError> {
            if req.method == "GET"
                && let Some(rest) = req.path.strip_prefix("/v1/registry/entities/")
                && let Some((kind, id)) = rest.split_once('/')
                && (kind == "template" || kind == "prompt")
            {
                let id = id.split('?').next().unwrap_or(id);
                return Ok(match reasoning::catalog::Catalog::bundled().get(&format!("{kind}:{id}")) {
                    Some(a) => Reply { status: 200, body: json!({"ref": {"kind": a.kind, "id": a.id, "version": a.version}, "content": {"id": a.id, "version": a.version, "locales": a.locales}}) },
                    None => Reply { status: 404, body: json!({"code": "not_found"}) },
                });
            }
            self.0.send(req)
        }
    }

    fn queja_loop(work: &Path, core: Arc<Core>, proven: &'static str) -> (ValueLoop, Arc<ByTarget>) {
        let by = Arc::new(ByTarget { proven, last: Mutex::new(String::new()), built: Mutex::new(vec![]) });
        let wrapped = Arc::new(PatchCore(core));
        let mut v = with_proof(work, Arc::new(Core::default()), Fake { refuse: false, story_outcome: "" });
        v.transport = wrapped.clone();
        v.proof.as_mut().unwrap().eval_transport = wrapped;
        v.proof.as_mut().unwrap().scripts = by.clone();
        std::fs::write(&v.cells, queja_cells()).unwrap();
        v.ports = candidate_ports();
        (v, by)
    }

    #[test]
    fn the_second_candidate_is_tried_only_when_the_first_is_not_proven_and_the_loop_stops_at_the_first_proven_one() {
        let work = temp("map1-second");
        let (v, by) = queja_loop(&work, Arc::new(Core::default()), "prompt:p/resumen_radicado");
        let out = v.run(&pulso::run::value_loop::NoPersist).unwrap();
        let rec = &out["findings"][0];
        assert_eq!(rec["dims"]["reason_category"], "Queja", "{rec}");
        assert_eq!((rec["outcome"].as_str(), rec["target_ref"].as_str()), (Some("announced"), Some("prompt:p/resumen_radicado")), "{rec}");
        let tried: Vec<_> = rec["attempts"].as_array().unwrap().iter().map(|a| (a["target_ref"].as_str().unwrap().to_string(), a["outcome"].as_str().unwrap().to_string())).collect();
        assert_eq!(tried, vec![("template:t/estado_pqr".to_string(), "not_announced:not_fixed".to_string()), ("prompt:p/resumen_radicado".to_string(), "announced".to_string())]);
        assert_eq!(*by.built.lock().unwrap(), vec!["template:t/estado_pqr", "prompt:p/resumen_radicado"]);
        let cands = rec["candidates"].as_array().unwrap();
        assert_eq!(cands.len(), 5, "Phone: the advisor copilot is a candidate too");
        assert_eq!(cands.iter().filter(|c| c["tried"] == true).count(), 2);
        assert!(cands.iter().all(|c| c["justification"].as_str().is_some_and(|j| !j.is_empty())));
        assert_eq!(cands[2]["not_tried_because"], "over_the_candidate_cap");
        assert_eq!(rec["mapping_claim"], "hypothesis_of_where_to_intervene_not_a_cause");
        assert_eq!(out["summary"]["announced"], 1);
    }

    #[test]
    fn a_proven_first_candidate_ends_the_finding_without_a_second_attempt() {
        let work = temp("map1-first");
        let (v, by) = queja_loop(&work, Arc::new(Core::default()), "template:t/estado_pqr");
        let out = v.run(&pulso::run::value_loop::NoPersist).unwrap();
        let rec = &out["findings"][0];
        assert_eq!((rec["outcome"].as_str(), rec["target_ref"].as_str()), (Some("announced"), Some("template:t/estado_pqr")), "{rec}");
        assert_eq!(rec["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(*by.built.lock().unwrap(), vec!["template:t/estado_pqr"]);
        assert_eq!(rec["candidates"][1]["not_tried_because"], "an_earlier_candidate_was_proven_or_the_finding_stopped");
    }

    #[test]
    fn at_most_two_candidates_are_tried_when_none_is_proven_and_the_cost_of_both_is_accounted() {
        let work = temp("map1-none");
        let (v, by) = queja_loop(&work, Arc::new(Core::default()), "nothing");
        let out = v.run(&pulso::run::value_loop::NoPersist).unwrap();
        let rec = &out["findings"][0];
        assert_eq!(rec["attempts"].as_array().unwrap().len(), 2, "{rec}");
        assert_eq!(by.built.lock().unwrap().len(), 2);
        assert!(rec["outcome"].as_str().unwrap().starts_with("not_announced:"));
        assert_eq!(out["summary"]["announced"], 0);
        let per_attempt: f64 = rec["attempts"].as_array().unwrap().iter().map(|a| a["cost_usd"].as_f64().unwrap_or(0.0)).sum();
        assert!((per_attempt - rec["metering"]["cost_usd"].as_f64().unwrap()).abs() < 1e-6, "the record cost is the sum of its attempts");
    }
}
