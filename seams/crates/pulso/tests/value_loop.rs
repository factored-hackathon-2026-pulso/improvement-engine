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
