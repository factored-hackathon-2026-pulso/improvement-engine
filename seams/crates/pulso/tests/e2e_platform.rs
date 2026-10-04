//! END TO END, no containers: platform-shaped data from the Python simulator (platform-sim/product_stream, scenarios
//! `escalation_rise` and `null`, 24 days of event_time) -> the spawned `pulso run` binary (product-sqlite adapter, the real
//! `monitor::tick` with the `rust-events` sensor, the engine job worker, `thread10::pipeline::run_signals`) -> the debug-api
//! store the console reads. The simulator is run with `uv run --python 3.12 python -m product_stream`; without `uv` the test says so
//! and returns (it is not silently green: the message is printed).
use debug_api::Store;
use serde_json::Value;
use sources::store::{FileStore, WatermarkStore};
use sources::{DataMode, SourceId, Watermark};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PULSO: &str = env!("CARGO_BIN_EXE_pulso");
const RUNNER: &str = env!("CARGO_BIN_EXE_pulso-synth-runner");
const EVENTS: u64 = 26_000;
const SOURCE: &str = "platform:sim";

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pulso-e2e-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Writes a simulated product SQLite (24 days of event_time) and returns its path, or `None` when `uv` is not installed.
fn simulate(dir: &Path, scenario: &str) -> Option<PathBuf> {
    let sim = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../platform-sim");
    let db = dir.join(format!("{scenario}.db"));
    let out = Command::new("uv")
        .current_dir(&sim)
        .args(["run", "--python", "3.12", "python", "-m", "product_stream", "--sqlite"])
        .arg(&db)
        .args(["--scenario", scenario, "--seed", "7", "--backfill", &EVENTS.to_string(), "--mean-gap-s", "900", "--batch", "1000"])
        .output();
    match out {
        Err(e) => {
            eprintln!("SKIPPED: `uv` is not available ({e}); the end-to-end test needs the Python simulator");
            None
        }
        Ok(o) if !o.status.success() => panic!("simulator failed: {}", String::from_utf8_lossy(&o.stderr)),
        Ok(_) => Some(db),
    }
}

struct Pulso {
    child: Child,
    addr: SocketAddr,
    lines: Arc<Mutex<Vec<Value>>>,
}

impl Drop for Pulso {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(db: &Path, work: &Path, store: &Path) -> Pulso {
    let mut c = Command::new(PULSO);
    for (k, _) in std::env::vars() {
        if k.starts_with("PULSO_") {
            c.env_remove(k);
        }
    }
    let mut child = c
        .arg("run")
        .arg("--exit-on-stdin-eof")
        .env("PULSO_STORAGE", "memory")
        .env("PULSO_DATA_MODE", "platform")
        .env("PULSO_SOURCE_ADAPTER", "product-sqlite")
        .env("PULSO_SOURCE_ID", SOURCE)
        .env("PULSO_SOURCE_SQLITE", db)
        .env("PULSO_SOURCE_PROVENANCE", "simulated")
        .env("PULSO_WORK_DIR", work)
        .env("PULSO_STORE_DIR", store)
        .env("PULSO_LISTEN_ADDR", "127.0.0.1:0")
        .env("PULSO_POLL_INTERVAL_MS", "100")
        .env("PULSO_READ_BATCH", "3000")
        .env("STEPS_RUNNER_EXE", RUNNER)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let lines: Arc<Mutex<Vec<Value>>> = Arc::default();
    let mut first = String::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    let addr = loop {
        first.clear();
        assert!(out.read_line(&mut first).unwrap() > 0, "pulso ended before listening");
        let v: Value = serde_json::from_str(first.trim()).unwrap_or_else(|_| panic!("stdout must be JSON lines: {first}"));
        lines.lock().unwrap().push(v.clone());
        if v["event"] == "listening" {
            break v["addr"].as_str().unwrap().parse().unwrap();
        }
        assert!(Instant::now() < deadline, "never listened");
    };
    let sink = lines.clone();
    std::thread::spawn(move || {
        let mut l = String::new();
        while out.read_line(&mut l).map(|n| n > 0).unwrap_or(false) {
            if let Ok(v) = serde_json::from_str::<Value>(l.trim()) {
                sink.lock().unwrap().push(v);
            }
            l.clear();
        }
    });
    Pulso { child, addr, lines }
}

fn get(addr: SocketAddr, path: &str) -> Value {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    serde_json::from_str(raw.split_once("\r\n\r\n").map_or("null", |(_, b)| b)).unwrap_or(Value::Null)
}

fn wait_until(secs: u64, what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(secs), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn watermark(work: &Path) -> Option<Watermark> {
    FileStore::open(work.join("watermarks")).unwrap().get(&SourceId::new(DataMode::Platform, SOURCE).unwrap()).unwrap().map(|r| r.watermark)
}

fn records(work: &Path) -> Vec<Value> {
    let mut v: Vec<Value> = std::fs::read_dir(work.join("runs")).map(|d| d.filter_map(Result::ok).map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap()).collect()).unwrap_or_default();
    v.sort_by_key(|r| r["watermark_to"].as_str().unwrap_or("").len().to_string() + r["watermark_to"].as_str().unwrap_or(""));
    v
}

fn stop(mut p: Pulso) -> i32 {
    drop(p.child.stdin.take()); // stdin EOF: the clean stop
    let t = Instant::now();
    loop {
        if let Some(st) = p.child.try_wait().unwrap() {
            return st.code().unwrap_or(-1);
        }
        assert!(t.elapsed() < Duration::from_secs(40), "pulso did not stop on stdin EOF");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Runs `pulso run` over the whole simulated source; returns (work dir, store dir) once every run is completed in the console store.
fn run_to_the_end(db: &Path, tag: &str) -> (PathBuf, PathBuf, Vec<Value>) {
    let dir = temp(tag);
    let (work, store) = (dir.join("work"), dir.join("store"));
    let p = spawn(db, &work, &store);
    wait_until(240, "the watermark at the end of the source", || watermark(&work) == Some(Watermark::Sequence(EVENTS as i64)));
    let recs = records(&work);
    wait_until(240, "every run completed in the console", || {
        let runs = get(p.addr, "/internal/v1/debug/runs");
        let items = runs["items"].as_array().cloned().unwrap_or_default();
        items.len() == recs.len() && items.iter().all(|r| r["state"] == "completed")
    });
    // idle ticks: nothing is read twice
    let before = records(&work).len();
    let idle_before = p.lines.lock().unwrap().iter().filter(|l| l["event"] == "monitor_tick" && l["processed"] == 0).count();
    wait_until(30, "idle monitor ticks", || p.lines.lock().unwrap().iter().filter(|l| l["event"] == "monitor_tick" && l["processed"] == 0).count() >= idle_before + 3);
    assert_eq!(records(&work).len(), before, "a tick with nothing new wrote no run");
    assert_eq!(stop(p), 0, "stdin EOF is a clean stop");
    (work, store, recs)
}

fn events_of(store: &Store, run: &str) -> Vec<Value> {
    store.events_after(run, 0, 1_000_000)
}

#[test]
fn escalation_rise_is_admitted_once_per_signal_with_a_verdict_and_the_console_panels_and_null_admits_nothing() {
    let dir = temp("sim");
    let (Some(esc), Some(null)) = (simulate(&dir, "escalation_rise"), simulate(&dir, "null")) else { return };

    // ---- escalation_rise: the planted cell is admitted ----
    let (work, store_dir, recs) = run_to_the_end(&esc, "esc");
    assert_eq!(watermark(&work), Some(Watermark::Sequence(EVENTS as i64)), "the watermark advanced to the end of the source");
    let store = Store::open(&store_dir).unwrap();
    let admitted: Vec<(&Value, Vec<String>)> = recs
        .iter()
        .map(|r| (r, r["sensor"]["output"]["signals"].as_array().unwrap().iter().map(|s| s["metric_id"].as_str().unwrap().to_string()).collect::<Vec<_>>()))
        .collect();
    let total: usize = admitted.iter().map(|(_, s)| s.len()).sum();
    assert!(total >= 1, "the planted pt/web_chat reassignment rise must be admitted at least once: {:?}", admitted.iter().map(|(r, s)| (r["run_id"].clone(), s.clone())).collect::<Vec<_>>());
    assert!(admitted.iter().all(|(_, s)| s.iter().all(|m| m == "reassignment_rate.pt.web_chat")), "no other cell is admitted: {admitted:?}");
    assert_eq!(store.runs().len(), recs.len(), "one console run per tick run");
    let mut verdicts_total = 0;
    let mut panels_seen = false;
    for (r, signals) in &admitted {
        let run = r["run_id"].as_str().unwrap();
        let evs = events_of(&store, run);
        let verdicts: Vec<&Value> = evs.iter().filter(|e| e["kind"] == "proposal_verdict").collect();
        assert_eq!(verdicts.len(), signals.len(), "{run}: one proposal and one ledger verdict per admitted signal");
        let ledger = std::fs::read_dir(work.join("pipeline").join(run).join("ledger")).map(|d| d.count()).unwrap_or(0);
        if !signals.is_empty() {
            assert!(ledger >= signals.len(), "{run}: the ledger holds one entry per proposal on disk");
        }
        for v in verdicts {
            verdicts_total += 1;
            let verdict = v["data"]["verdict"].as_str().unwrap();
            assert!(["not_viable", "not_evaluable"].contains(&verdict), "the offline Core double never makes a proposal viable: {v}");
            assert_eq!(v["data"]["signal_id"], "reassignment_rate.pt.web_chat");
        }
        let state = store.state(run).unwrap();
        assert_eq!(state["run"]["state"], "completed");
        assert!(state["run"]["title"].as_str().unwrap().contains("simulated"), "labelled: {}", state["run"]["title"]);
        if !signals.is_empty() {
            panels_seen = true;
            assert!(state["investigation"].is_object() && state["gates"].is_object(), "{run}: investigation and gates panels are populated");
            assert!(evs.iter().any(|e| e["kind"] == "investigation_set") && evs.iter().any(|e| e["kind"] == "gates_set"));
            let ids: Vec<&str> = state["doubles"].as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap()).collect();
            assert!(ids.iter().any(|i| i.starts_with("data.source")) && ids.iter().any(|i| i.starts_with("port.core")), "doubles are declared: {ids:?}");
        }
    }
    assert_eq!(verdicts_total, total);
    assert!(panels_seen);

    // ---- a restart over the same work dir reads nothing and adds nothing ----
    let events_before: usize = store.runs().iter().map(|r| events_of(&store, r).len()).sum();
    let p = spawn(&esc, &work, &store_dir);
    wait_until(30, "idle ticks after the restart", || p.lines.lock().unwrap().iter().filter(|l| l["event"] == "monitor_tick" && l["processed"] == 0).count() >= 3);
    assert_eq!(stop(p), 0);
    assert_eq!(records(&work).len(), recs.len(), "the persisted watermark: nothing is read twice across a restart");
    let again = Store::open(&store_dir).unwrap();
    assert_eq!(again.runs().iter().map(|r| events_of(&again, r).len()).sum::<usize>(), events_before, "no duplicate events after the restart");

    // ---- null: nothing is admitted, nothing is proposed ----
    let (work, store_dir, recs) = run_to_the_end(&null, "null");
    assert!(recs.iter().all(|r| r["sensor"]["output"]["signals"].as_array().unwrap().is_empty()), "the null scenario admits no signal");
    let store = Store::open(&store_dir).unwrap();
    assert_eq!(store.runs().len(), recs.len());
    for run in store.runs() {
        assert!(!events_of(&store, &run).iter().any(|e| e["kind"] == "proposal_verdict"), "{run}: no proposal for a null stream");
        assert!(store.state(&run).unwrap()["run"]["title"].as_str().unwrap().contains("0 signal"));
    }
    assert!(!work.join("pipeline").exists() || std::fs::read_dir(work.join("pipeline")).unwrap().count() == 0, "no pipeline ran");
}
