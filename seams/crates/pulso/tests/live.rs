//! `pulso demo`: the ten-step thread streams into a RunEventSink WHILE it runs; the HTTP sink writes to a real debug-api.
use debug_api::{App, Config, NewEvent, RunEventSink, Store};
use pulso::live::{DemoOpts, demo};
use pulso::sink::HttpSink;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Records every emit in order next to the store it forwards to.
struct Tap(Arc<Store>, Mutex<Vec<(String, String)>>);
impl RunEventSink for Tap {
    fn emit(&self, run: &str, ev: NewEvent) -> Result<Value, String> {
        self.1.lock().unwrap().push((ev.kind.clone(), ev.entity_id.clone() + "|" + ev.data["node"]["label"].as_str().unwrap_or("")));
        self.0.emit(run, ev)
    }
}

fn opts(name: &str, pace: u64) -> DemoOpts {
    let work = std::env::temp_dir().join(format!("pulso-live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let mut o = DemoOpts::new("run-demo0-test".into(), work, env!("CARGO_BIN_EXE_pulso-synth-runner").into());
    o.pace = Duration::from_millis(pace);
    o
}

#[test]
fn streams_steps_while_the_job_runs_and_ends_completed_with_doubles_and_gate() {
    let store = Arc::new(Store::memory());
    let tap = Arc::new(Tap(store.clone(), Mutex::default()));
    let run = demo(tap.clone(), &opts("a", 0)).expect("demo");
    assert_eq!(run.error, None);
    let log = tap.1.lock().unwrap().clone();
    assert_eq!(log[0].0, "run_started");
    assert_eq!(log.last().unwrap().0, "run_state_changed");
    let pos = |needle: &str| log.iter().position(|(_, e)| e.contains(needle)).unwrap_or_else(|| panic!("{needle} in {log:?}"));
    assert!(pos("signals|signals [pending]") < pos("signals|signals [stand-in]"), "pending first, then the label");
    assert!(pos("signals|signals [stand-in]") < pos("gate|gate [stand-in]") && pos("gate|gate [stand-in]") < pos("publish|publish [stand-in]"), "step order follows the job");
    let gates = log.iter().position(|(k, _)| k == "gates_set").unwrap();
    assert!(pos("publish|publish [stand-in]") < gates, "gates after the steps");
    let st = store.state("run-demo0-test").unwrap();
    assert_eq!(st["run"]["state"], "completed");
    let nodes = st["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 12, "ten report steps, two of them split");
    let label = |id: &str| nodes.iter().find(|n| n["node_id"] == id).unwrap()["label"].as_str().unwrap().to_string();
    assert_eq!(label("recompute"), "recompute [real-narrow]");
    assert_eq!(label("revision"), "revision [not_exercised]");
    assert_eq!(label("approval"), "approval [simulated]");
    assert!(nodes.iter().all(|n| n["status"] != "running" && n["status"] != "queued"), "nothing is left running");
    let doubles: Vec<&str> = st["doubles"].as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert!(doubles.iter().any(|d| d.starts_with("port.core:offline-double")), "{doubles:?}");
    assert!(doubles.iter().any(|d| d.starts_with("gate.override:")), "{doubles:?}");
    assert_eq!(st["gates"]["improvement"]["status"], "fail", "the improvement gate failed on the committed arms");
    let first_doubles = log.iter().position(|(k, _)| k == "doubles_declared").unwrap();
    assert!(first_doubles < pos("signals|signals [pending]") + 20, "doubles are declared up front, not only at the end");
}

#[test]
fn pace_spreads_the_run_over_time() {
    let store = Arc::new(Store::memory());
    let t = Instant::now();
    demo(store, &opts("b", 40)).expect("demo");
    assert!(t.elapsed() >= Duration::from_millis(9 * 40), "{:?}", t.elapsed());
}

#[test]
fn a_stopped_job_is_a_failed_run_never_completed() {
    let store = Arc::new(Store::memory());
    let mut o = opts("c", 0);
    o.denied_kind = true;
    let run = demo(store.clone(), &o).expect("demo");
    assert!(run.error.is_some());
    let st = store.state("run-demo0-test").unwrap();
    assert_eq!(st["run"]["state"], "failed");
    let n = |id: &str| st["nodes"].as_array().unwrap().iter().find(|n| n["node_id"] == id).unwrap()["label"].as_str().unwrap().to_string();
    assert!(n("compile").starts_with("compile [blocked("), "{}", n("compile"));
    assert_eq!(n("publish"), "publish [not_exercised]");
}

#[test]
fn http_sink_appends_to_a_real_server_and_refuses_a_wrong_token() {
    let store = Arc::new(Store::memory());
    let server = debug_api::server::bind_loopback("127.0.0.1:0").unwrap();
    let addr = server.server_addr().to_ip().unwrap();
    let app = Arc::new(App::new(store.clone(), Config { admin_token: Some("tok".into()), ..Config::default() }));
    std::thread::spawn(move || debug_api::server::serve(server, app));
    let sink = HttpSink::new(&addr.to_string(), "tok");
    sink.emit("run-x", NewEvent::new("run_started", "run", "run-x", json!({"title": "X", "state": "running", "origin": "manual"}))).expect("emit");
    assert_eq!(store.state("run-x").unwrap()["run"]["title"], "X");
    let bad = HttpSink::new(&addr.to_string(), "nope");
    assert!(bad.emit("run-x", NewEvent::new("run_state_changed", "run", "run-x", json!({"state": "completed"}))).unwrap_err().contains("401"));
    let off = HttpSink::new("127.0.0.1:1", "tok");
    assert!(off.emit("run-x", NewEvent::new("run_started", "run", "run-x", json!({}))).is_err());
}

/// Every panel event, in order, with the node labels, so a test can say which step had committed when it appeared.
fn kinds(tap: &Tap) -> Vec<String> {
    tap.1.lock().unwrap().iter().map(|(k, e)| format!("{k}|{e}")).collect()
}

#[test]
fn panels_fill_as_each_step_commits_not_only_at_the_end() {
    let store = Arc::new(Store::memory());
    let tap = Arc::new(Tap(store.clone(), Mutex::default()));
    demo(tap.clone(), &opts("panels", 0)).expect("demo");
    let log = kinds(&tap);
    let first = |needle: &str| log.iter().position(|l| l.starts_with(needle) || l.contains(needle)).unwrap_or_else(|| panic!("{needle} in {log:?}"));
    assert!(first("investigation_set") < first("compile|compile [stand-in]"), "the investigation appears once validation committed, before compile completes: {log:?}");
    assert!(first("diff_set") < first("gate|gate [stand-in]"), "the diff appears when compile committed, before the gate: {log:?}");
    assert!(first("gates_set") < first("approval|approval [simulated]"), "gate details appear when the gate committed: {log:?}");
    assert!(first("decision_set") < first("publish|publish [stand-in]"), "the decision card appears when authority committed, before publish: {log:?}");
    let count = |k: &str| log.iter().filter(|l| l.starts_with(&format!("{k}|"))).count();
    assert!(count("decision_set") == 1 && count("diff_set") == 1, "unchanged panels are not re-sent: {log:?}");
    assert!(count("gates_set") <= 2 && count("investigation_set") <= 3, "{log:?}");

    let st = store.state("run-demo0-test").unwrap();
    assert!(st["investigation"]["hypothesis"].as_str().unwrap().contains("Scripted scout claims"));
    assert_eq!(st["investigation"]["verifier"], "corroborated");
    assert!(st["diff"]["lines"].to_string().contains("prompt:resumen_radicado@2"));
    assert_eq!(st["decision"]["card"]["label"], "SIMULATED");
    assert_eq!(st["gates"]["combined"]["decision"], "override_simulated_human", "the final gates carry the authority outcome");
    assert_eq!(st["gates"]["native"]["status"], "pass");
    assert_eq!(st["alternatives"].as_array().unwrap().len(), 2);
    assert!(st["nodes"].as_array().unwrap().iter().all(|n| n["trace_id"].is_null()), "no trace id is ever invented");
}

#[test]
fn a_blocked_approval_streams_a_blocked_decision_card_and_no_diff_when_compile_was_denied() {
    let store = Arc::new(Store::memory());
    let mut o = opts("blocked", 0);
    o.human_override = false;
    demo(store.clone(), &o).expect("demo");
    let st = store.state("run-demo0-test").unwrap();
    assert_eq!(st["decision"]["card"]["state"], "blocked", "{}", st["decision"]);
    assert!(st["decision"]["card"]["reasons"][0].as_str().unwrap().contains("blocked(gate)"));
    let store = Arc::new(Store::memory());
    let mut o = opts("denied", 0);
    o.denied_kind = true;
    demo(store.clone(), &o).expect("demo");
    let st = store.state("run-demo0-test").unwrap();
    assert!(st["diff"].is_null() && st["decision"].is_null(), "no proposal, no decision: nothing is invented");
    assert!(st["investigation"]["hypotheses"][1]["verdict"].as_str().unwrap().starts_with("blocked("));
}
