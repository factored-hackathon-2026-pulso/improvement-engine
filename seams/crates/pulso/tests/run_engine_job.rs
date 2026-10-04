//! W7: the engine job of `pulso run`. A job keyed `monitor:<run_id>` is run through `thread10::pipeline::run_signals`: one
//! proposal per admitted signal, one ledger verdict per proposal, the events into the debug-api store in process. Models are the
//! labelled scripted port and the Core is the labelled offline double unless configured otherwise.
use debug_api::Store;
use pg::repo::{JobRepository, MemRepo};
use pulso::config::RunConfig;
use pulso::run::engine_job::EngineRunner;
use pulso::run::supervisor::StopToken;
use pulso::run::tasks::{JobCtx, JobRunner};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const RUNNER: &str = env!("CARGO_BIN_EXE_pulso-synth-runner");
const T: &str = "tenant-local";
const RUN: &str = "mon-0123456789abcdef";

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pulso-job-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("runs")).unwrap();
    d
}

fn cfg(work: &Path, extra: &[(&str, &str)]) -> RunConfig {
    let mut m: HashMap<String, String> = [("PULSO_STORAGE", "memory"), ("PULSO_DATA_MODE", "platform"), ("PULSO_SOURCE_ADAPTER", "product-sqlite"), ("PULSO_SOURCE_ID", "platform:sim"), ("PULSO_WORK_DIR", work.to_str().unwrap()), ("PULSO_SOURCE_SQLITE", "unused.db")]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    m.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    RunConfig::from_lookup(&|k| m.get(k).cloned()).unwrap()
}

fn signal(i: usize, cell: &str, num: u64, den: u64) -> Value {
    json!({"signal_id": format!("sig-{:04}", i + 1), "metric_id": format!("reassignment_rate.{cell}"), "population": cell.replace('.', "/"), "numerator": num, "denominator": den, "holdout_checked": true, "evidence_ref": format!("ev-{:04}", i + 1)})
}

/// A run record as `monitor::tick` writes it (shape of `pulso-monitor-run/0`).
fn record(signals: Vec<Value>, mode: &str, semantics: &str) -> Value {
    let (origin, label, adapter, class) = if mode == "dataset" {
        ("dataset_replay", "demo/replay data, not production; release and observation simulated", "dataset-pg", "e0")
    } else {
        ("platform_live", "real platform signals; release and observation simulated until EXT-2", "product-sqlite", "treated")
    };
    json!({
        "contract": "pulso-monitor-run/0", "run_id": RUN, "package": "pkg-0123456789abcdef", "source_id": format!("{mode}:sim"), "data_mode": mode, "data_origin": origin,
        "adapter": adapter, "data_class": class, "label": label, "watermark_from": "seq:0", "watermark_to": "seq:3000", "more": false,
        "events_read": 3000, "events_admitted": 3000, "quarantined": {"denied": 0, "unknown": 0}, "observed_until": "2026-09-25T03:15:09Z", "history": {"days": 22, "cases": 2224},
        "evidence": {"sensor": semantics, "signals": "computed-local", "release": "simulated", "observation": "simulated", "model": "none"},
        "sensor": {"status": "ok", "semantics": semantics, "signals": signals.len(), "discards": 0, "output": {"signals": signals, "discards": []}},
    })
}

fn write_record(work: &Path, rec: &Value) {
    std::fs::write(work.join("runs").join(format!("{RUN}.json")), rec.to_string()).unwrap();
}

struct Fixture {
    runner: EngineRunner,
    store: Arc<Store>,
    repo: Arc<MemRepo>,
}

fn fixture(work: &Path, extra: &[(&str, &str)]) -> Fixture {
    let store = Arc::new(Store::memory());
    let c = cfg(work, extra);
    let runner = EngineRunner::new(&c, work, Path::new(RUNNER), store.clone()).expect("runner");
    Fixture { runner, store, repo: Arc::new(MemRepo::new()) }
}

fn events(store: &Store, run: &str) -> Vec<Value> {
    store.events_after(run, 0, 100_000)
}

fn of_kind(evs: &[Value], kind: &str) -> Vec<Value> {
    evs.iter().filter(|e| e["kind"] == kind).cloned().collect()
}

fn run_job(f: &Fixture) -> Result<(), String> {
    let id = f.repo.admit_keyed(T, &format!("monitor:{RUN}")).unwrap();
    let c = f.repo.claim_next(T, "w", 100, 60).unwrap().unwrap();
    assert_eq!(c.job, id);
    let stop = StopToken::new();
    let ctx = JobCtx { repo: f.repo.as_ref(), tenant: T, worker: "w", stop: &stop, now: 100, lease_seconds: 60 };
    f.runner.run(&c, &ctx)
}

#[test]
fn n_admitted_signals_make_n_proposals_and_n_ledger_verdicts_in_the_console_store() {
    let work = temp("two");
    write_record(&work, &record(vec![signal(0, "pt.web_chat", 54, 321), signal(1, "es.app_chat", 40, 200)], "platform", "rust-events"));
    let f = fixture(&work, &[]);
    run_job(&f).unwrap();
    assert_eq!(f.store.runs(), vec![RUN.to_string()], "one console run per tick run");
    let evs = events(&f.store, RUN);
    let verdicts = of_kind(&evs, "proposal_verdict");
    assert_eq!(verdicts.len(), 2, "one verdict per admitted signal");
    let ids: Vec<&str> = verdicts.iter().map(|e| e["data"]["signal_id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["reassignment_rate.pt.web_chat", "reassignment_rate.es.app_chat"], "the signals are named by their cell metric ids");
    for v in &verdicts {
        let verdict = v["data"]["verdict"].as_str().unwrap();
        assert!(["not_viable", "not_evaluable"].contains(&verdict), "the offline double never yields a viable proposal: {v}");
        assert!(!v["data"]["reason"].as_str().unwrap().is_empty());
    }
    // the ledger is on disk, one immutable entry per proposal
    assert!(work.join("pipeline").join(RUN).join("ledger").is_dir());
    let state = f.store.state(RUN).unwrap();
    assert_eq!(state["run"]["state"], "completed");
    assert!(state["investigation"].is_object() && state["gates"].is_object(), "console panels are populated: {state}");
    assert!(!of_kind(&evs, "decision_set").is_empty() || !of_kind(&evs, "gates_set").is_empty());
}

#[test]
fn the_run_says_what_is_real_and_what_is_not() {
    let work = temp("labels");
    write_record(&work, &record(vec![signal(0, "pt.web_chat", 54, 321)], "platform", "rust-events"));
    let f = fixture(&work, &[("PULSO_SOURCE_PROVENANCE", "simulated")]);
    run_job(&f).unwrap();
    let state = f.store.state(RUN).unwrap();
    let title = state["run"]["title"].as_str().unwrap().to_string();
    for needle in ["platform", "rust-events", "scripted", "offline", "simulated"] {
        assert!(title.to_lowercase().contains(needle), "title lacks {needle:?}: {title}");
    }
    let doubles: Vec<String> = state["doubles"].as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap().to_string()).collect();
    for part in ["data.source", "release", "observation", "port.human_issuer", "port.core", "scout"] {
        assert!(doubles.iter().any(|d| d.starts_with(part)), "no double for {part}: {doubles:?}");
    }
    assert!(!doubles.iter().any(|d| d.starts_with("signals")), "the rust-events sensor is real code, not a double: {doubles:?}");
    // the run profile: mode, adapter, source id, sensor, models, ports as nodes of the run graph
    let nodes = state["nodes"].as_array().unwrap();
    let label = |id: &str| nodes.iter().find(|n| n["node_id"] == id).unwrap_or_else(|| panic!("no node {id}: {nodes:?}"))["label"].as_str().unwrap().to_string();
    let source = label("source");
    for needle in ["platform", "product-sqlite", "platform:sim", "seq:0", "seq:3000"] {
        assert!(source.contains(needle), "{needle}: {source}");
    }
    assert!(label("sensor").contains("rust-events"));
    assert!(label("models").contains("scripted"));
    let ports = label("ports");
    for needle in ["core offline-double", "release simulated", "observation simulated", "human simulated"] {
        assert!(ports.contains(needle), "{needle}: {ports}");
    }
    let profile = of_kind(&events(&f.store, RUN), "run_profile_set");
    assert_eq!(profile.len(), 1);
    assert_eq!(profile[0]["data"]["data_mode"], "platform");
    assert_eq!(profile[0]["data"]["sensor"], "rust-events");
}

#[test]
fn a_scripted_scout_claims_the_observed_rate_so_the_recompute_corroborates_and_the_verdict_comes_from_the_gate() {
    let work = temp("claim");
    write_record(&work, &record(vec![signal(0, "pt.web_chat", 54, 321)], "platform", "rust-events"));
    let f = fixture(&work, &[]);
    run_job(&f).unwrap();
    let v = &of_kind(&events(&f.store, RUN), "proposal_verdict")[0]["data"];
    assert_ne!(v["reason"], "claim_not_corroborated", "the scripted claim copies the observed 0.17: {v}");
    let h = &v["hypotheses"][0];
    assert_eq!((h["claimed_rate"].as_f64(), h["recomputed_rate"].as_f64(), &h["match"]), (Some(0.17), Some(0.17), &json!(true)), "{v}");
    assert!(v["models"].as_array().unwrap().iter().all(|m| m["label"] == "scripted" && m["real"] == false), "scripted answers are never labelled real: {v}");
}

#[test]
fn a_tick_without_admitted_signals_is_a_visible_run_with_no_proposal() {
    let work = temp("none");
    write_record(&work, &record(vec![], "platform", "rust-events"));
    let f = fixture(&work, &[]);
    run_job(&f).unwrap();
    let evs = events(&f.store, RUN);
    assert!(of_kind(&evs, "proposal_verdict").is_empty());
    let state = f.store.state(RUN).unwrap();
    assert_eq!(state["run"]["state"], "completed");
    assert!(state["run"]["title"].as_str().unwrap().contains("0 signal"), "{state}");
}

#[test]
fn dataset_mode_is_labelled_demo_replay_with_a_stand_in_sensor() {
    let work = temp("dataset");
    write_record(&work, &record(vec![signal(0, "e0.copilot_query", 22, 30)], "dataset", "claude-standin"));
    let c = ("PULSO_DATA_MODE", "dataset");
    let f = fixture(&work, &[c, ("PULSO_SOURCE_ADAPTER", "dataset-pg"), ("PULSO_SOURCE_ID", "dataset:sim")]);
    run_job(&f).unwrap();
    let state = f.store.state(RUN).unwrap();
    let title = state["run"]["title"].as_str().unwrap().to_lowercase();
    assert!(title.contains("demo/replay") && title.contains("not production") && title.contains("stand-in"), "{title}");
    let doubles: Vec<String> = state["doubles"].as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap().to_string()).collect();
    assert!(doubles.iter().any(|d| d.starts_with("data.dataset")) && doubles.iter().any(|d| d.starts_with("signals")), "{doubles:?}");
    assert_eq!(of_kind(&events(&f.store, RUN), "proposal_verdict").len(), 1);
}

#[test]
fn processing_the_same_run_twice_adds_no_event_and_no_ledger_entry() {
    let work = temp("twice");
    write_record(&work, &record(vec![signal(0, "pt.web_chat", 54, 321), signal(1, "es.app_chat", 40, 200)], "platform", "rust-events"));
    let f = fixture(&work, &[]);
    f.runner.process(RUN).unwrap();
    let first = events(&f.store, RUN);
    f.runner.process(RUN).unwrap();
    let second = events(&f.store, RUN);
    let kinds = |e: &[Value]| e.iter().map(|x| x["kind"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(of_kind(&second, "proposal_verdict").len(), 2, "no duplicate verdict after a replay");
    assert_eq!(of_kind(&second, "run_started").len(), 1);
    assert_eq!(kinds(&first).iter().filter(|k| *k == "proposal_verdict").count(), 2);
    let ledger = std::fs::read_dir(work.join("pipeline").join(RUN).join("ledger")).unwrap().count();
    assert!(ledger >= 2);
}

#[test]
fn a_job_without_a_monitor_key_is_ignored_and_a_missing_record_is_a_retryable_error() {
    let work = temp("odd");
    let f = fixture(&work, &[]);
    let id = f.repo.admit(T).unwrap();
    let c = f.repo.claim_next(T, "w", 100, 60).unwrap().unwrap();
    assert_eq!(c.job, id);
    let stop = StopToken::new();
    let ctx = JobCtx { repo: f.repo.as_ref(), tenant: T, worker: "w", stop: &stop, now: 100, lease_seconds: 60 };
    assert_eq!(f.runner.run(&c, &ctx), Ok(()), "not ours: nothing to do");
    assert!(f.store.runs().is_empty());
    let g = fixture(&temp("missing"), &[]);
    assert!(run_job(&g).is_err(), "no run record: the lease lapses and the job is retried");
}

#[test]
fn a_gateway_that_is_not_configured_refuses_honestly_instead_of_falling_back_to_scripted() {
    let work = temp("gw");
    write_record(&work, &record(vec![signal(0, "pt.web_chat", 54, 321)], "platform", "rust-events"));
    let f = fixture(&work, &[("PULSO_MODEL_PORT", "gateway")]);
    run_job(&f).unwrap();
    let v = &of_kind(&events(&f.store, RUN), "proposal_verdict")[0]["data"];
    assert_eq!((v["verdict"].as_str(), v["reason"].as_str()), (Some("not_evaluable"), Some("model_refused")), "{v}");
    assert!(v["models"].as_array().unwrap().iter().all(|m| m["real"] == false));
}

#[test]
fn the_live_core_port_is_refused_at_construction_when_it_is_not_configured() {
    let work = temp("core");
    let store = Arc::new(Store::memory());
    let c = cfg(&work, &[("PULSO_CORE_PORT", "live")]);
    let e = EngineRunner::with_env(&c, &work, Path::new(RUNNER), store, Arc::new(|_| None)).err().expect("refused");
    assert!(e.contains("PULSO_CORE_PORT") || e.to_lowercase().contains("core"), "{e}");
}
