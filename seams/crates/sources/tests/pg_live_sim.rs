//! Live end-to-end against a database populated by the product simulator (platform-sim `product_stream --postgres`)
//! on the REAL `db/sql` schema: simulator -> `product` schema -> product-postgres adapter (read-only role) -> package ->
//! watermark in Postgres (`pulso_app`). Skipped unless PULSO_E2E_RO_DSN and PULSO_E2E_APP_DSN (keyword DSNs, never
//! printed) are set; PULSO_REQUIRE_POSTGRES=1 turns a missing one into a failure.
//! Run with `--test-threads=1`: both tests read the same event_log.
mod common;
use postgres::{Config, NoTls};
use sources::config::Config as MonCfg;
use sources::monitor::{TickOutcome, tick};
use sources::pg_product::PostgresProduct;
use sources::pg_store::PgStore;
use sources::store::{WatermarkRecord, WatermarkStore};
use sources::{DataMode, SourceAdapter, SourceError, SourceId, Watermark};
use std::sync::atomic::{AtomicBool, Ordering};

fn dsns() -> Option<(String, String)> {
    match (std::env::var("PULSO_E2E_RO_DSN"), std::env::var("PULSO_E2E_APP_DSN")) {
        (Ok(a), Ok(b)) => Some((a, b)),
        _ => {
            assert!(std::env::var("PULSO_REQUIRE_POSTGRES").is_err(), "PULSO_E2E_*_DSN not set");
            eprintln!("SKIP: PULSO_E2E_RO_DSN / PULSO_E2E_APP_DSN not set (simulator end-to-end)");
            None
        }
    }
}

fn id(n: &str) -> SourceId {
    SourceId::new(DataMode::Platform, &format!("platform:{n}")).unwrap()
}

fn cfg(name: &str, cap: &str) -> (MonCfg, std::path::PathBuf) {
    let work = common::temp_path(name).join("work");
    let c = MonCfg::from_pairs(&[("data_mode", "platform"), ("adapter", "product-postgres"), ("source_id", &format!("platform:{name}")), ("work_dir", work.to_str().unwrap()), ("runner_exe", env!("CARGO_BIN_EXE_sources-synth-runner")), ("batch_cap", cap)]).unwrap();
    (c, work)
}

/// A store that reads for real but "dies" (errors) instead of committing, once.
struct DiesBeforeCommit<'a> {
    inner: &'a PgStore,
    armed: AtomicBool,
}
impl WatermarkStore for DiesBeforeCommit<'_> {
    fn get(&self, id: &SourceId) -> Result<Option<WatermarkRecord>, SourceError> {
        self.inner.get(id)
    }
    fn commit(&self, id: &SourceId, e: Option<&Watermark>, r: &WatermarkRecord) -> Result<(), SourceError> {
        if self.armed.swap(false, Ordering::SeqCst) {
            return Err(SourceError::Io("killed between read and commit".into()));
        }
        self.inner.commit(id, e, r)
    }
}

fn event_log_stats(ro: &str) -> (i64, i64) {
    let mut c = ro.parse::<Config>().unwrap().connect(NoTls).unwrap();
    let r = c.query_one("SELECT count(*), coalesce(max(sequence), 0) FROM product.event_log", &[]).unwrap();
    (r.get(0), r.get(1))
}

#[test]
fn simulator_data_ticks_to_the_end_then_reads_nothing_and_survives_a_kill_before_commit() {
    let Some((ro, app)) = dsns() else { return };
    let (total, max_seq) = event_log_stats(&ro);
    assert!(total > 20 && total == max_seq, "event_log must be a dense simulator backfill: {total}/{max_seq}");
    let (c, work) = cfg("pglive-kill", "40");
    let a = PostgresProduct::connect(&ro, "product", id("pglive-kill")).unwrap();
    assert!(a.schema_check().unwrap().ok(), "real db/sql schema must satisfy the adapter");
    let store = PgStore::connect(&app).unwrap();
    assert!(store.get(&id("pglive-kill")).unwrap().is_none(), "start from a clean watermark (reset the table between runs)");

    // 1. first batch is read and processed, then the process "dies" before the watermark commit
    let dying = DiesBeforeCommit { inner: &store, armed: AtomicBool::new(true) };
    assert!(tick(&c, &a, &dying).is_err());
    assert!(store.get(&id("pglive-kill")).unwrap().is_none(), "nothing committed by the dead run");
    let runs = || std::fs::read_dir(work.join("runs")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect::<Vec<_>>();
    let orphan = runs();
    assert_eq!(orphan.len(), 1);

    // 2. restart with fresh connections: the same batch is re-read and replays onto the same run id
    let a2 = PostgresProduct::connect(&ro, "product", id("pglive-kill")).unwrap();
    let store2 = PgStore::connect(&app).unwrap();
    let first = tick(&c, &a2, &store2).unwrap();
    let TickOutcome::Processed { run_id, events, from, to, .. } = first else { panic!("expected a batch") };
    assert_eq!(events, 40);
    assert_eq!((from, to), (Watermark::Sequence(0), Watermark::Sequence(40)));
    assert_eq!(runs(), orphan, "replay produced the identical run record, no second run");
    assert_eq!(orphan[0], format!("{run_id}.json"));

    // 3. drain to the end
    let mut seen = events;
    let mut batches = 1;
    while let TickOutcome::Processed { events, .. } = tick(&c, &a2, &store2).unwrap() {
        seen += events;
        batches += 1;
    }
    assert_eq!(seen as i64, total, "every event exactly once across the kill");
    let w = store2.get(&id("pglive-kill")).unwrap().unwrap();
    assert_eq!(w.watermark, Watermark::Sequence(max_seq));
    assert_eq!(w.batches, batches);
    // 4. second tick reads nothing and leaves the watermark alone
    assert!(matches!(tick(&c, &a2, &store2).unwrap(), TickOutcome::Idle { .. }));
    assert_eq!(store2.get(&id("pglive-kill")).unwrap().unwrap(), w);
    eprintln!("LIVE kill/resume: events={seen} batches={batches} watermark={max_seq}");
}

#[test]
fn ticks_follow_a_simulator_that_is_still_writing() {
    let Some((ro, app)) = dsns() else { return };
    let Ok(stop) = std::env::var("PULSO_E2E_STOP_FILE") else {
        eprintln!("SKIP: PULSO_E2E_STOP_FILE not set (needs a running `--follow` simulator)");
        return;
    };
    let (c, _w) = cfg("pglive-follow", "30");
    let a = PostgresProduct::connect(&ro, "product", id("pglive-follow")).unwrap();
    let store = PgStore::connect(&app).unwrap();
    let (mut seen, mut idle_after_stop) = (0usize, 0);
    let started = std::time::Instant::now();
    let mut stopped = false;
    while idle_after_stop < 3 {
        match tick(&c, &a, &store).unwrap() {
            TickOutcome::Processed { events, .. } => seen += events,
            TickOutcome::Idle { .. } => {
                if stopped {
                    idle_after_stop += 1;
                }
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
        }
        if !stopped && started.elapsed().as_secs() >= 8 {
            std::fs::write(&stop, "x").unwrap();
            stopped = true;
        }
        assert!(started.elapsed().as_secs() < 90, "did not converge");
    }
    let (total, max_seq) = event_log_stats(&ro);
    assert_eq!(total, max_seq, "event_log.sequence is dense");
    assert_eq!(seen as i64, total, "followed every row exactly once while it was being written");
    assert_eq!(store.get(&id("pglive-follow")).unwrap().unwrap().watermark, Watermark::Sequence(max_seq));
    eprintln!("LIVE follow: events={seen} final_sequence={max_seq} secs={}", started.elapsed().as_secs());
}
