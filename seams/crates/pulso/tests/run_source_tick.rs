//! W7: the `pulso run` monitor task reads through the real `sources::monitor::tick` with the adapter the config names, writes
//! packages and run records under the work dir, and hands each run to the engine job queue (keyed by the run id) BEFORE the
//! watermark moves.
use pg::repo::{JobRepository, MemRepo};
use pulso::config::RunConfig;
use pulso::run::source::{SourceTick, build_tick};
use pulso::run::tasks::{Tick, TickCtx};
use sources::store::{FileStore, WatermarkRecord, WatermarkStore};
use sources::{SourceError, SourceId, Watermark};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const RUNNER: &str = env!("CARGO_BIN_EXE_pulso-synth-runner");
const T: &str = "tenant-local";

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pulso-src-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Synthetic platform-shaped SQLite (invented values, 7 events in 1 day: the cold-start gate of the sensor holds).
fn fixture(dir: &Path) -> PathBuf {
    let p = dir.join("platform.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute_batch(
        "CREATE TABLE event_log(sequence INTEGER PRIMARY KEY, event_id TEXT, event_type TEXT, entity TEXT, entity_id TEXT, case_id TEXT, actor_role TEXT, actor_id TEXT, event_time TEXT, ingested_at TEXT, payload TEXT, tenant_id TEXT);
         CREATE TABLE cases(id TEXT PRIMARY KEY, customer_id TEXT, channel TEXT, language TEXT, priority TEXT, opened_at TEXT, sla_due_at TEXT, previous_case_id TEXT, tenant_id TEXT);",
    )
    .unwrap();
    for n in 1..=7 {
        let t = if n % 3 == 0 { "case.closed" } else { "case.opened" };
        c.execute("INSERT INTO event_log VALUES(?,?,?,?,?,?,?,?,?,?,?,?)", rusqlite::params![n, format!("ev-{n}"), t, "case", format!("c{n}"), format!("c{n}"), "customer", "a", "2026-10-01T10:00:00Z", "x", "SECRET-PAYLOAD", "t"]).unwrap();
    }
    p
}

fn cfg(work: &Path, db: &Path, extra: &[(&str, &str)]) -> RunConfig {
    let mut m: HashMap<String, String> = [
        ("PULSO_STORAGE", "memory"),
        ("PULSO_DATA_MODE", "platform"),
        ("PULSO_SOURCE_ADAPTER", "product-sqlite"),
        ("PULSO_SOURCE_ID", "platform:sim"),
        ("PULSO_WORK_DIR", work.to_str().unwrap()),
        ("PULSO_SOURCE_SQLITE", db.to_str().unwrap()),
        ("STEPS_RUNNER_EXE", RUNNER),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    m.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    RunConfig::from_lookup(&|k| m.get(k).cloned()).unwrap()
}

fn ctx(c: &RunConfig) -> TickCtx {
    TickCtx { data_mode: c.data_mode, adapter: c.adapter.clone(), batch_cap: c.batch_cap }
}

fn tick_of(c: &RunConfig, repo: Arc<MemRepo>) -> Box<dyn Tick> {
    build_tick(c, repo, T, Path::new(RUNNER)).expect("tick")
}

fn queued(repo: &MemRepo) -> Vec<String> {
    let mut keys = vec![];
    while let Some(j) = repo.claim_next(T, "probe", 1, 30).unwrap() {
        keys.push(repo.job_key(T, &j.job).unwrap().unwrap_or_default());
    }
    keys
}

#[test]
fn a_tick_reads_the_source_writes_the_run_and_enqueues_one_keyed_job() {
    let dir = temp("one");
    let (work, db) = (dir.join("work"), fixture(&dir));
    let c = cfg(&work, &db, &[]);
    let repo = Arc::new(MemRepo::new());
    let r = tick_of(&c, repo.clone()).tick(&ctx(&c)).unwrap();
    assert_eq!(r.processed, 7, "events read");
    let runs: Vec<_> = std::fs::read_dir(work.join("runs")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(runs.len(), 1, "{runs:?}");
    let run_id = runs[0].trim_end_matches(".json").to_string();
    assert!(work.join("packages").join(run_id.replace("mon-", "pkg-")).join("manifest.json").is_file());
    assert_eq!(queued(&repo), vec![format!("monitor:{run_id}")], "exactly one job, keyed by the run id");
    let wm = FileStore::open(work.join("watermarks")).unwrap().get(&SourceId::new(sources::DataMode::Platform, "platform:sim").unwrap()).unwrap().unwrap();
    assert_eq!(wm.watermark, Watermark::Sequence(7), "the watermark advanced");
}

#[test]
fn a_second_tick_reads_nothing_and_enqueues_nothing() {
    let dir = temp("idle");
    let (work, db) = (dir.join("work"), fixture(&dir));
    let c = cfg(&work, &db, &[]);
    let repo = Arc::new(MemRepo::new());
    let mut t = tick_of(&c, repo.clone());
    assert_eq!(t.tick(&ctx(&c)).unwrap().processed, 7);
    assert_eq!(t.tick(&ctx(&c)).unwrap().processed, 0);
    assert_eq!(queued(&repo).len(), 1);
}

#[test]
fn a_backlog_larger_than_the_read_batch_is_drained_in_one_tick_without_rereading() {
    let dir = temp("backlog");
    let (work, db) = (dir.join("work"), fixture(&dir));
    let c = cfg(&work, &db, &[("PULSO_READ_BATCH", "3")]);
    let repo = Arc::new(MemRepo::new());
    let mut t = tick_of(&c, repo.clone());
    assert_eq!(t.tick(&ctx(&c)).unwrap().processed, 7);
    assert_eq!(queued(&repo).len(), 3, "3 + 3 + 1 events: three runs, three jobs");
    assert_eq!(t.tick(&ctx(&c)).unwrap().processed, 0);
}

/// Fails the first watermark commit, as a kill between "read" and "commit" would leave it.
struct KillOnce {
    inner: FileStore,
    killed: std::sync::atomic::AtomicBool,
}
impl WatermarkStore for KillOnce {
    fn get(&self, id: &SourceId) -> Result<Option<WatermarkRecord>, SourceError> {
        self.inner.get(id)
    }
    fn commit(&self, id: &SourceId, expected: Option<&Watermark>, rec: &WatermarkRecord) -> Result<(), SourceError> {
        if !self.killed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(SourceError::Io("killed before the watermark commit".into()));
        }
        self.inner.commit(id, expected, rec)
    }
}

#[test]
fn a_kill_between_read_and_watermark_commit_resumes_without_a_duplicate_job_or_run() {
    let dir = temp("kill");
    let (work, db) = (dir.join("work"), fixture(&dir));
    let c = cfg(&work, &db, &[]);
    let repo = Arc::new(MemRepo::new());
    let store = KillOnce { inner: FileStore::open(work.join("watermarks")).unwrap(), killed: Default::default() };
    let mut first = SourceTick::with_store(&c, repo.clone(), T, Path::new(RUNNER), Box::new(store)).unwrap();
    assert!(first.tick(&ctx(&c)).is_err(), "the commit failed");
    let id = SourceId::new(sources::DataMode::Platform, "platform:sim").unwrap();
    assert!(FileStore::open(work.join("watermarks")).unwrap().get(&id).unwrap().is_none(), "nothing committed");
    assert_eq!(repo.job_key(T, "job-0").unwrap().as_deref().map(|k| k.starts_with("monitor:mon-")), Some(true), "the job was handed over before the commit");
    // a fresh process: same work dir, same queue
    let mut second = tick_of(&c, repo.clone());
    assert_eq!(second.tick(&ctx(&c)).unwrap().processed, 7);
    assert_eq!(queued(&repo).len(), 1, "the replayed batch has the same run id: still one job");
    assert_eq!(std::fs::read_dir(work.join("runs")).unwrap().count(), 1, "still one run record");
}

#[test]
fn the_stub_adapter_gives_the_stub_tick_and_a_real_adapter_missing_its_dsn_is_refused_by_name() {
    let dir = temp("cfg");
    let stub = RunConfig::from_lookup(&|k| [("PULSO_STORAGE", "memory"), ("PULSO_DATA_MODE", "dataset")].iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())).unwrap();
    let mut t = build_tick(&stub, Arc::new(MemRepo::new()), T, Path::new(RUNNER)).unwrap();
    assert_eq!(t.tick(&ctx(&stub)).unwrap().processed, 0);
    let m: HashMap<&str, String> = [("PULSO_STORAGE", "memory".to_string()), ("PULSO_DATA_MODE", "dataset".into()), ("PULSO_SOURCE_ADAPTER", "dataset-pg".into()), ("PULSO_WORK_DIR", dir.display().to_string())].into_iter().collect();
    let ds = RunConfig::from_lookup(&|k| m.get(k).cloned()).unwrap();
    let mut t = build_tick(&ds, Arc::new(MemRepo::new()), T, Path::new(RUNNER)).unwrap();
    let e = t.tick(&ctx(&ds)).expect_err("no DSN, no connection attempt");
    assert!(e.contains("PULSO_PG_DATASET_DSN"), "{e}");
    assert!(!e.contains("postgres://"));
}
