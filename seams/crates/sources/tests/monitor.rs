mod common;
use common::*;
use serde_json::Value;
use sources::config::Config;
use sources::monitor::{TickOutcome, tick};
use sources::sqlite::SqliteProduct;
use sources::store::{FileStore, WatermarkRecord, WatermarkStore};
use sources::{Batch, DataMode, PlatformEvent, Row, SchemaReport, SourceAdapter, SourceError, SourceId, Watermark};
use std::path::{Path, PathBuf};

const RUNNER: &str = env!("CARGO_BIN_EXE_sources-synth-runner");

fn cfg(work: &Path, id: &str, cap: &str) -> Config {
    Config::from_pairs(&[
        ("data_mode", "platform"),
        ("adapter", "product-sqlite"),
        ("source_id", id),
        ("work_dir", work.to_str().unwrap()),
        ("runner_exe", RUNNER),
        ("batch_cap", cap),
    ])
    .unwrap()
}
fn adapter(db: &Path, id: &str) -> SqliteProduct {
    SqliteProduct::open(db, SourceId::new(DataMode::Platform, id).unwrap()).unwrap()
}
fn runs(work: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(work.join("runs")).map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect()).unwrap_or_default();
    v.sort();
    v
}
fn record(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}
fn processed(o: TickOutcome) -> (String, usize, Value) {
    match o {
        TickOutcome::Processed { run_id, events, record, .. } => (run_id, events, record),
        other => panic!("expected Processed, got {other:?}"),
    }
}

#[test]
fn first_tick_builds_a_package_runs_the_sensor_and_records_provenance() {
    let dir = temp_path("tick1");
    let db = fixture(&dir, "tenant-a");
    let work = dir.join("work");
    let store = FileStore::open(work.join("wm")).unwrap();
    let c = cfg(&work, "platform:tenant-a", "100");
    let (run_id, n, rec) = processed(tick(&c, &adapter(&db, "platform:tenant-a"), &store).unwrap());
    assert_eq!(n, 7);
    assert_eq!(rec["run_id"], run_id.as_str());
    assert_eq!(rec["data_mode"], "platform");
    assert_eq!(rec["data_origin"], "platform_live");
    assert_eq!(rec["source_id"], "platform:tenant-a");
    assert_eq!(rec["adapter"], "product-sqlite");
    assert_eq!(rec["data_class"], "treated");
    assert_eq!(rec["watermark_from"], "seq:0");
    assert_eq!(rec["watermark_to"], "seq:7");
    assert_eq!(rec["evidence"]["release"], "simulated");
    assert_eq!(rec["evidence"]["observation"], "simulated");
    assert_eq!(rec["evidence"]["sensor"], "claude-standin");
    assert!(rec["label"].as_str().unwrap().contains("release and observation simulated"));
    assert!(!rec["label"].as_str().unwrap().contains("demo/replay"));
    assert_eq!(rec["sensor"]["status"], "ok");
    // the sensor job was committed through the engine job store, the run record and package exist on disk
    assert_eq!(runs(&work).len(), 1);
    assert!(work.join("jobs").join(&run_id).join("out").exists() || work.join("jobs").join(&run_id).read_dir().unwrap().count() > 0);
    let pkg = work.join("packages").join(rec["package"].as_str().unwrap());
    let lines = std::fs::read_to_string(pkg.join("events.ndjson")).unwrap();
    assert_eq!(lines.lines().count(), 7);
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(pkg.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["source_id"], "platform:tenant-a");
    assert_eq!(manifest["events"], 7);
    // watermark committed; nothing denied anywhere on disk
    assert_eq!(store.get(&SourceId::new(DataMode::Platform, "platform:tenant-a").unwrap()).unwrap().unwrap().watermark, Watermark::Sequence(7));
    for s in SENTINELS {
        assert!(!lines.contains(s) && !rec.to_string().contains(s), "{s}");
    }
}

#[test]
fn second_tick_with_nothing_new_is_idle_and_does_not_rerun() {
    let dir = temp_path("idle");
    let db = fixture(&dir, "tenant-a");
    let work = dir.join("work");
    let store = FileStore::open(work.join("wm")).unwrap();
    let c = cfg(&work, "platform:tenant-a", "100");
    let a = adapter(&db, "platform:tenant-a");
    tick(&c, &a, &store).unwrap();
    assert!(matches!(tick(&c, &a, &store).unwrap(), TickOutcome::Idle { .. }));
    assert_eq!(runs(&work).len(), 1);
    append_events(&db, 8, 2);
    let (_, n, rec) = processed(tick(&c, &a, &store).unwrap());
    assert_eq!((n, rec["watermark_from"].as_str(), rec["watermark_to"].as_str()), (2, Some("seq:7"), Some("seq:9")));
    assert_eq!(runs(&work).len(), 2);
}

#[test]
fn batch_cap_bounds_every_tick_and_no_event_is_read_twice() {
    let dir = temp_path("cap");
    let db = fixture(&dir, "tenant-a");
    let work = dir.join("work");
    let store = FileStore::open(work.join("wm")).unwrap();
    let c = cfg(&work, "platform:tenant-a", "3");
    let a = adapter(&db, "platform:tenant-a");
    let mut sizes = vec![];
    while let TickOutcome::Processed { events, .. } = tick(&c, &a, &store).unwrap() {
        sizes.push(events);
    }
    assert_eq!(sizes, [3, 3, 1]);
}

#[test]
fn cold_start_reports_insufficient_history_and_computes_history_free_signals() {
    let dir = temp_path("cold");
    let db = fixture(&dir, "tenant-a");
    let work = dir.join("work");
    let store = FileStore::open(work.join("wm")).unwrap();
    let (_, _, rec) = processed(tick(&cfg(&work, "platform:tenant-a", "100"), &adapter(&db, "platform:tenant-a"), &store).unwrap());
    assert_eq!(rec["history"]["days"], 0);
    assert_eq!(rec["history"]["cases"], 2);
    let free = rec["signals"]["history_free"].as_array().unwrap();
    let get = |n: &str| free.iter().find(|s| s["name"] == n).unwrap_or_else(|| panic!("{n}"));
    assert_eq!(get("cases_opened")["value"], 2);
    assert_eq!(get("cases_closed")["value"], 1);
    assert_eq!(get("turns_created")["value"], 2);
    assert_eq!(get("cases_opened")["weak"], true, "n=7 events but support below min_support is flagged only when n < 5; opened=2 is weak");
    let dep = rec["signals"]["history_dependent"].as_array().unwrap();
    assert!(dep.len() >= 3);
    for s in dep {
        assert_eq!(s["status"], "insufficient_history", "{s}");
        assert_eq!(s["need"]["days"], 14);
        assert_eq!(s["need"]["cases"], 200);
        assert_eq!(s["have"]["cases"], 2);
        assert!(s.get("value").is_none());
    }
}

#[test]
fn history_gate_opens_only_with_enough_days_and_cases() {
    let dir = temp_path("warm");
    let db = fixture(&dir, "tenant-a");
    let work = dir.join("work");
    let store = FileStore::open(work.join("wm")).unwrap();
    let id = SourceId::new(DataMode::Platform, "platform:tenant-a").unwrap();
    let seed = WatermarkRecord { watermark: Watermark::Sequence(0), adapter: "product-sqlite".into(), first_event_time: Some("2026-08-01T00:00:00Z".into()), cases_opened: 500, batches: 9 };
    store.commit(&id, None, &seed).unwrap();
    let (_, _, rec) = processed(tick(&cfg(&work, "platform:tenant-a", "100"), &adapter(&db, "platform:tenant-a"), &store).unwrap());
    for s in rec["signals"]["history_dependent"].as_array().unwrap() {
        assert_ne!(s["status"], "insufficient_history", "{s}");
    }
    assert_eq!(store.get(&id).unwrap().unwrap().batches, 10);
    assert_eq!(store.get(&id).unwrap().unwrap().cases_opened, 502);
    assert_eq!(store.get(&id).unwrap().unwrap().first_event_time.as_deref(), Some("2026-08-01T00:00:00Z"));
}

#[test]
fn denied_and_unknown_event_types_are_quarantined_not_packaged() {
    let dir = temp_path("quar");
    let db = fixture(&dir, "tenant-a");
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        for (s, t) in [(8, "auth.password_accepted"), (9, "totally.unknown"), (10, "staff.shift_started")] {
            c.execute("INSERT INTO event_log(sequence,event_id,event_type,event_time) VALUES(?,?,?,?)", rusqlite::params![s, format!("ev-{s}"), t, "2026-10-01T12:00:00Z"]).unwrap();
        }
    }
    let work = dir.join("work");
    let store = FileStore::open(work.join("wm")).unwrap();
    let (_, n, rec) = processed(tick(&cfg(&work, "platform:tenant-a", "100"), &adapter(&db, "platform:tenant-a"), &store).unwrap());
    assert_eq!(n, 10);
    assert_eq!(rec["events_admitted"], 7);
    assert_eq!(rec["quarantined"]["denied"], 1);
    assert_eq!(rec["quarantined"]["unknown"], 2);
    let pkg = work.join("packages").join(rec["package"].as_str().unwrap());
    let lines = std::fs::read_to_string(pkg.join("events.ndjson")).unwrap();
    assert_eq!(lines.lines().count(), 7);
    assert!(!lines.contains("password_accepted") && !lines.contains("unknown"));
}

struct FailFirstCommit<'a> {
    inner: &'a FileStore,
    failed: std::cell::Cell<bool>,
}
impl WatermarkStore for FailFirstCommit<'_> {
    fn get(&self, id: &SourceId) -> Result<Option<WatermarkRecord>, SourceError> {
        self.inner.get(id)
    }
    fn commit(&self, id: &SourceId, e: Option<&Watermark>, r: &WatermarkRecord) -> Result<(), SourceError> {
        if !self.failed.replace(true) {
            return Err(SourceError::Io("simulated kill between read and watermark commit".into()));
        }
        self.inner.commit(id, e, r)
    }
}

#[test]
fn crash_between_read_and_watermark_commit_replays_the_batch_idempotently() {
    let dir = temp_path("crash");
    let db = fixture(&dir, "tenant-a");
    let work = dir.join("work");
    let inner = FileStore::open(work.join("wm")).unwrap();
    let store = FailFirstCommit { inner: &inner, failed: std::cell::Cell::new(false) };
    let id = SourceId::new(DataMode::Platform, "platform:tenant-a").unwrap();
    let a = adapter(&db, "platform:tenant-a");
    let c = cfg(&work, "platform:tenant-a", "100");
    assert!(tick(&c, &a, &store).is_err());
    assert_eq!(inner.get(&id).unwrap(), None, "watermark must not advance when the commit did not happen");
    let first = runs(&work);
    assert_eq!(first.len(), 1, "the batch was processed before the kill");
    let before = std::fs::read_to_string(&first[0]).unwrap();
    // restart: the runner is gone, so any attempt to RE-RUN the sensor would fail; success proves the committed output is reused
    let mut c2 = c.clone();
    c2.runner_exe = work.join("no-such-runner.exe");
    let (run_id, n, _) = processed(tick(&c2, &a, &store).unwrap());
    assert_eq!(n, 7, "at-least-once: the same batch is read again");
    assert_eq!(runs(&work), first, "same deterministic run id, no second run");
    assert_eq!(std::fs::read_to_string(&first[0]).unwrap(), before, "identical record");
    assert!(first[0].file_name().unwrap().to_string_lossy().starts_with(&run_id));
    assert_eq!(inner.get(&id).unwrap().unwrap().watermark, Watermark::Sequence(7));
    assert!(matches!(tick(&c2, &a, &store).unwrap(), TickOutcome::Idle { .. }));
}

#[test]
fn two_sources_never_mix_in_watermarks_runs_or_packages() {
    let dir = temp_path("two");
    let da = fixture(&temp_path("two-a"), "tenant-a");
    let db = fixture(&temp_path("two-b"), "tenant-b");
    append_events(&db, 8, 3);
    let work = dir.join("work");
    let store = FileStore::open(work.join("wm")).unwrap();
    let (ca, cb) = (cfg(&work, "platform:tenant-a", "100"), cfg(&work, "platform:tenant-b", "100"));
    let (aa, ab) = (adapter(&da, "platform:tenant-a"), adapter(&db, "platform:tenant-b"));
    let (ra, na, reca) = processed(tick(&ca, &aa, &store).unwrap());
    assert_eq!(store.get(aa.source_id()).unwrap().unwrap().watermark, Watermark::Sequence(7));
    assert_eq!(store.get(ab.source_id()).unwrap(), None, "reading a never touches b");
    let (rb, nb, recb) = processed(tick(&cb, &ab, &store).unwrap());
    assert_eq!((na, nb), (7, 10));
    assert_ne!(ra, rb);
    assert_eq!((reca["source_id"].as_str(), recb["source_id"].as_str()), (Some("platform:tenant-a"), Some("platform:tenant-b")));
    assert_eq!(store.get(aa.source_id()).unwrap().unwrap().watermark, Watermark::Sequence(7));
    assert_eq!(store.get(ab.source_id()).unwrap().unwrap().watermark, Watermark::Sequence(10));
    // a run is bound to exactly one source: config and adapter must name the same one, in the same mode
    assert!(matches!(tick(&ca, &ab, &store), Err(SourceError::Mismatch(_))));
    let dataset_cfg = Config::from_pairs(&[("data_mode", "dataset"), ("adapter", "dataset-pg"), ("source_id", "dataset:e0:b1"), ("work_dir", work.to_str().unwrap()), ("runner_exe", RUNNER)]).unwrap();
    assert!(matches!(tick(&dataset_cfg, &aa, &store), Err(SourceError::Mismatch(_))));
}

/// Dataset-mode double: the tick is generic over the port, so the same code labels a replay honestly.
struct FakeDataset(SourceId);
impl SourceAdapter for FakeDataset {
    fn source_id(&self) -> &SourceId {
        &self.0
    }
    fn adapter(&self) -> &'static str {
        "dataset-pg"
    }
    fn data_class(&self) -> &'static str {
        "e0"
    }
    fn list_tables(&self) -> Result<Vec<String>, SourceError> {
        Ok(vec![])
    }
    fn schema_check(&self) -> Result<SchemaReport, SourceError> {
        Ok(SchemaReport::default())
    }
    fn read_events(&self, after: &Watermark, _l: usize) -> Result<Batch, SourceError> {
        let ev = |i: i64, t: &str| PlatformEvent { sequence: None, event_id: format!("e{i}"), event_type: t.into(), entity: Some("case".into()), entity_id: Some(format!("c{i}")), case_id: Some(format!("c{i}")), actor_role: None, actor_id: None, event_time: "2026-09-01T10:00:00Z".into() };
        let next = Watermark::Dataset { ingested_at: "2026-10-01 00:00:00+00".into(), batch_id: "b1".into(), key: "e0_case:c2".into() };
        if after == &next {
            return Ok(Batch { events: vec![], next, more: false });
        }
        Ok(Batch { events: vec![ev(1, "case.opened"), ev(2, "case.closed")], next, more: false })
    }
    fn read_dimension(&self, _t: &str, _l: usize) -> Result<Vec<Row>, SourceError> {
        Ok(vec![])
    }
}

#[test]
fn dataset_mode_is_labelled_demo_replay_never_production() {
    let dir = temp_path("ds");
    let work = dir.join("work");
    let store = FileStore::open(work.join("wm")).unwrap();
    let c = Config::from_pairs(&[("data_mode", "dataset"), ("adapter", "dataset-pg"), ("source_id", "dataset:e0:b1"), ("work_dir", work.to_str().unwrap()), ("runner_exe", RUNNER)]).unwrap();
    let a = FakeDataset(SourceId::new(DataMode::Dataset, "dataset:e0:b1").unwrap());
    let (_, _, rec) = processed(tick(&c, &a, &store).unwrap());
    assert_eq!(rec["data_mode"], "dataset");
    assert_eq!(rec["data_origin"], "dataset_replay");
    assert_eq!(rec["data_class"], "e0");
    assert_eq!(rec["adapter"], "dataset-pg");
    let label = rec["label"].as_str().unwrap();
    assert!(label.contains("demo/replay") && label.contains("not production"));
    assert_eq!(rec["evidence"]["release"], "simulated");
    assert!(rec["watermark_to"].as_str().unwrap().starts_with("ds:"));
    assert!(matches!(tick(&c, &a, &store).unwrap(), TickOutcome::Idle { .. }));
}

#[test]
fn a_failing_sensor_never_advances_the_watermark() {
    let dir = temp_path("sensorfail");
    let db = fixture(&dir, "tenant-a");
    let work = dir.join("work");
    let store = FileStore::open(work.join("wm")).unwrap();
    let mut c = cfg(&work, "platform:tenant-a", "100");
    c.runner_exe = work.join("missing-runner.exe");
    let a = adapter(&db, "platform:tenant-a");
    assert!(matches!(tick(&c, &a, &store), Err(SourceError::Sensor(_))));
    assert_eq!(store.get(a.source_id()).unwrap(), None);
}
