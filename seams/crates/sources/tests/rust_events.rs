//! R1G wiring: `sensor=rust-events` behind the same `monitor::tick`. Event packages come from a product-sqlite source
//! built from the R1S-shaped generator (shared with the steps tests; planted effect in the pt/web_chat cell).
#[path = "../../steps/tests/ev_common/mod.rs"]
mod ev_common;
mod common;
use common::temp_path;
use ev_common::{Scn, make};
use rusqlite::Connection;
use serde_json::Value;
use sources::config::Config;
use sources::monitor::{TickOutcome, tick};
use sources::sqlite::SqliteProduct;
use sources::store::FileStore;
use sources::{DataMode, SourceId};
use std::path::{Path, PathBuf};

const RUNNER: &str = env!("CARGO_BIN_EXE_sources-synth-runner");
const ID: &str = "platform:r1g";

fn build_db(dir: &Path, scn: Scn, seed: u64, cases: usize) -> PathBuf {
    let p = dir.join("product.db");
    let c = Connection::open(&p).unwrap();
    c.execute_batch(
        "CREATE TABLE event_log(sequence INTEGER PRIMARY KEY, event_id TEXT NOT NULL, event_type TEXT NOT NULL, entity TEXT, entity_id TEXT,
           case_id TEXT, actor_role TEXT, actor_id TEXT, event_time TEXT NOT NULL, ingested_at TEXT, payload TEXT, tenant_id TEXT);
         CREATE TABLE cases(id TEXT PRIMARY KEY, customer_id TEXT, channel TEXT, language TEXT, priority TEXT, opened_at TEXT, sla_due_at TEXT,
           previous_case_id TEXT, tenant_id TEXT);",
    )
    .unwrap();
    let d = make(scn, seed, cases);
    let tx = c.unchecked_transaction().unwrap();
    for l in d.events.lines() {
        let v: Value = serde_json::from_str(l).unwrap();
        tx.execute(
            "INSERT INTO event_log(sequence,event_id,event_type,entity,entity_id,case_id,actor_role,actor_id,event_time) VALUES(?,?,?,?,?,?,?,?,?)",
            rusqlite::params![v["sequence"].as_i64(), v["event_id"].as_str(), v["event_type"].as_str(), v["entity"].as_str(), v["entity_id"].as_str(), v["case_id"].as_str(), v["actor_role"].as_str(), v["actor_id"].as_str(), v["event_time"].as_str()],
        )
        .unwrap();
    }
    for l in d.cases.lines() {
        let v: Value = serde_json::from_str(l).unwrap();
        tx.execute(
            "INSERT INTO cases(id,customer_id,channel,language,priority,previous_case_id) VALUES(?,?,?,?,?,?)",
            rusqlite::params![v["case_id"].as_str(), "CUS-x", v["channel"].as_str(), v["language"].as_str(), v["priority"].as_str(), v["previous_case_id"].as_str()],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    p
}

fn cfg(work: &Path, extra: &[(&str, &str)]) -> Config {
    let mut pairs = vec![
        ("data_mode", "platform"),
        ("adapter", "product-sqlite"),
        ("source_id", ID),
        ("work_dir", work.to_str().unwrap()),
        ("runner_exe", RUNNER),
        ("batch_cap", "5000"),
        ("min_history_days", "0"),
    ];
    pairs.extend_from_slice(extra);
    Config::from_pairs(&pairs).unwrap()
}

fn run_all(db: &Path, c: &Config, work: &Path) -> Vec<Value> {
    let a = SqliteProduct::open(db, SourceId::new(DataMode::Platform, ID).unwrap()).unwrap();
    let store = FileStore::open(work.join("wm")).unwrap();
    let mut out = vec![];
    while let TickOutcome::Processed { record, more, .. } = tick(c, &a, &store).unwrap() {
        out.push(record);
        if !more {
            break;
        }
    }
    out
}

#[test]
fn platform_mode_defaults_to_rust_events_and_the_key_is_validated() {
    let w = temp_path("cfg");
    assert_eq!(cfg(&w, &[]).sensor.as_str(), "rust-events");
    assert_eq!(cfg(&w, &[("sensor", "stand-in")]).sensor.as_str(), "stand-in");
    let bad = Config::from_pairs(&[("data_mode", "platform"), ("adapter", "product-sqlite"), ("source_id", ID), ("work_dir", "x"), ("runner_exe", "y"), ("sensor", "magic")]);
    assert!(bad.is_err());
}

#[test]
fn tick_with_rust_events_admits_the_planted_cell_and_records_what_it_did_not_do() {
    let dir = temp_path("planted");
    let db = build_db(&dir, Scn::Escalation, 1, 3000);
    let work = dir.join("work");
    let recs = run_all(&db, &cfg(&work, &[("sensor", "rust-events")]), &work);
    assert!(recs.len() >= 2, "several batches: {}", recs.len());
    let last = recs.last().unwrap();
    assert_eq!(last["evidence"]["sensor"], "rust-events");
    assert_eq!(last["sensor"]["semantics"], "rust-events");
    let sigs = last["sensor"]["output"]["signals"].as_array().unwrap();
    assert_eq!(sigs.len(), 1, "{}", last["sensor"]);
    assert_eq!(sigs[0]["population"], "pt/web_chat");
    assert_eq!(sigs[0]["holdout_checked"], true);
    assert_eq!(last["sensor"]["report"]["contract"], "sensor-events/1");
    assert!(last["sensor"]["report"]["not_done"].as_array().unwrap().iter().any(|s| s.as_str().unwrap().contains("payload")));
    let text = last.to_string();
    assert!(!text.contains("CAS-") && !text.contains("CUS-") && !text.contains("STF-"), "no identifiers in the run record");
}

#[test]
fn cumulative_history_first_batches_are_cold_start_not_signals() {
    let dir = temp_path("cold");
    let db = build_db(&dir, Scn::Escalation, 1, 3000);
    let work = dir.join("work");
    let recs = run_all(&db, &cfg(&work, &[("sensor", "rust-events"), ("batch_cap", "400")]), &work);
    let first = &recs[0]["sensor"];
    assert_eq!(first["output"]["signals"].as_array().unwrap().len(), 0);
    let report = &first["report"];
    assert!(report["discards"].as_array().unwrap().iter().all(|d| d["reason"] == "insufficient_history"), "{report}");
}

#[test]
fn null_scenario_through_the_tick_admits_nothing() {
    let dir = temp_path("null");
    let db = build_db(&dir, Scn::Null, 7, 3000);
    let work = dir.join("work");
    let recs = run_all(&db, &cfg(&work, &[]), &work);
    assert!(recs.iter().all(|r| r["sensor"]["output"]["signals"].as_array().unwrap().is_empty()));
}

#[test]
fn packages_carry_only_allow_listed_dimension_columns() {
    let dir = temp_path("pkg");
    let db = build_db(&dir, Scn::Escalation, 1, 400);
    let work = dir.join("work");
    let recs = run_all(&db, &cfg(&work, &[]), &work);
    let pkg = work.join("packages").join(recs[0]["package"].as_str().unwrap());
    let line = std::fs::read_to_string(pkg.join("cases.ndjson")).unwrap();
    let first: Value = serde_json::from_str(line.lines().next().unwrap()).unwrap();
    let mut keys: Vec<&str> = first.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["case_id", "channel", "language", "previous_case_id", "priority"]);
    assert!(!line.contains("CUS-"), "customer ids never reach the package");
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(pkg.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["dimensions"][0], "cases.ndjson");
}

#[test]
fn stand_in_remains_selectable_and_unchanged() {
    let dir = temp_path("standin");
    let db = build_db(&dir, Scn::Escalation, 1, 300);
    let work = dir.join("work");
    let recs = run_all(&db, &cfg(&work, &[("sensor", "stand-in")]), &work);
    assert_eq!(recs[0]["evidence"]["sensor"], "claude-standin");
    assert_eq!(recs[0]["sensor"]["semantics"], "claude-standin");
    assert!(!work.join("packages").join(recs[0]["package"].as_str().unwrap()).join("cases.ndjson").exists());
}
