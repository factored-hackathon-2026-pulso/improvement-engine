//! Live Postgres tests: skipped unless PULSO_TEST_PG_ADMIN (an admin DSN of a throw-away server) is set; with
//! PULSO_REQUIRE_POSTGRES=1 a missing server is a failure. Everything lives in a throw-away database created here.
mod common;
use postgres::{Client, Config, NoTls};
use sources::config::Config as MonCfg;
use sources::monitor::{TickOutcome, tick};
use sources::pg_dataset::DatasetPg;
use sources::pg_product::PostgresProduct;
use sources::pg_store::PgStore;
use sources::store::{WatermarkRecord, WatermarkStore};
use sources::{DataMode, SourceAdapter, SourceError, SourceId, Watermark};
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

struct Db {
    admin: Config,
    name: String,
    ro_dsn: String,
    app_dsn: String,
}

impl Db {
    fn create() -> Option<Db> {
        let Ok(dsn) = std::env::var("PULSO_TEST_PG_ADMIN") else {
            assert!(std::env::var("PULSO_REQUIRE_POSTGRES").is_err(), "PULSO_TEST_PG_ADMIN not set");
            eprintln!("SKIP: PULSO_TEST_PG_ADMIN not set (live Postgres adapter tests)");
            return None;
        };
        let admin: Config = dsn.parse().expect("admin dsn");
        let tag = format!("{}_{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst));
        let name = format!("r1m_{tag}");
        admin.connect(NoTls).unwrap().batch_execute(&format!("CREATE DATABASE {name}")).unwrap();
        let mut c = admin.clone();
        c.dbname(&name);
        let mut c = c.connect(NoTls).unwrap();
        let (ro, app) = (format!("r1m_ro_{tag}"), format!("r1m_app_{tag}"));
        let mig = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../migrations/0053_pulso_source_watermark.sql")).unwrap();
        c.batch_execute(&format!(
            "CREATE ROLE {ro} LOGIN PASSWORD 'test-{tag}'; CREATE ROLE {app} LOGIN PASSWORD 'test-{tag}';
             ALTER ROLE {ro} SET default_transaction_read_only = on;
             CREATE SCHEMA product; CREATE SCHEMA raw;
             CREATE TABLE product.event_log(sequence bigint PRIMARY KEY, event_id text, event_type text, entity text, entity_id text, case_id text, actor_role text, actor_id text, event_time timestamptz, ingested_at timestamptz, payload jsonb, tenant_id text);
             CREATE TABLE product.login_accounts(id text, password_hash text);
             INSERT INTO product.login_accounts VALUES('l1','SECRET-PW-HASH');
             INSERT INTO product.event_log SELECT n, 'ev-'||n, CASE WHEN n % 3 = 0 THEN 'case.closed' ELSE 'case.opened' END, 'case', 'c'||n, 'c'||n, 'customer', 'a', '2026-10-01T10:00:00Z'::timestamptz + n * interval '1 minute', now(), '{{\"t\":\"SECRET-PAYLOAD\"}}', 't' FROM generate_series(1, 7) n;
             GRANT USAGE ON SCHEMA product, raw TO {ro};
             GRANT SELECT (sequence, event_id, event_type, entity, entity_id, case_id, actor_role, actor_id, event_time) ON product.event_log TO {ro};
             CREATE TABLE raw.e0_case(case_id text, opened_at timestamptz, _batch_id text, _source_file text, _ingested_at timestamptz);
             CREATE TABLE raw.e0_turn(turn_id text, case_id text, event_time timestamptz, author_role text, text text, _batch_id text, _source_file text, _ingested_at timestamptz);
             CREATE TABLE raw.e0_case_close(case_id text, closed_at timestamptz, _batch_id text, _source_file text, _ingested_at timestamptz);
             INSERT INTO raw.e0_case SELECT 'c'||n, '2026-09-01T10:00:00Z', 'b1', 'f', '2026-10-01T00:00:00Z' FROM generate_series(1,3) n;
             INSERT INTO raw.e0_turn SELECT 't'||n, 'c'||n, '2026-09-01T10:01:00Z', 'customer', 'SECRET-TURN-TEXT', 'b1', 'f', '2026-10-01T00:00:00Z' FROM generate_series(1,3) n;
             INSERT INTO raw.e0_case_close SELECT 'c'||n, '2026-09-01T11:00:00Z', 'b2', 'f', '2026-10-01T00:00:00Z' FROM generate_series(1,2) n;
             GRANT SELECT (case_id, opened_at, _batch_id, _ingested_at) ON raw.e0_case TO {ro};
             GRANT SELECT (turn_id, case_id, event_time, author_role, _batch_id, _ingested_at) ON raw.e0_turn TO {ro};
             GRANT SELECT (case_id, closed_at, _batch_id, _ingested_at) ON raw.e0_case_close TO {ro};"
        ))
        .unwrap();
        c.batch_execute(&mig).unwrap();
        c.batch_execute(&format!("GRANT USAGE ON SCHEMA pulso TO {app}; GRANT SELECT, INSERT, UPDATE ON pulso.source_watermark TO {app};")).unwrap();
        let host = match admin.get_hosts().first() {
            Some(postgres::config::Host::Tcp(h)) => h.clone(),
            _ => "localhost".into(),
        };
        let port = admin.get_ports().first().copied().unwrap_or(5432);
        let dsn = |u: &str| format!("host={host} port={port} user={u} password=test-{tag} dbname={name}");
        let (ro_dsn, app_dsn) = (dsn(&ro), dsn(&app));
        Some(Db { admin, name, ro_dsn, app_dsn })
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        if let Ok(mut a) = self.admin.connect(NoTls) {
            let _ = a.batch_execute(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name));
        }
    }
}

fn pid(n: &str) -> SourceId {
    SourceId::new(DataMode::Platform, &format!("platform:{n}")).unwrap()
}

#[test]
fn product_postgres_is_read_only_allow_listed_and_follows_the_watermark() {
    let Some(db) = Db::create() else { return };
    let a = PostgresProduct::connect(&db.ro_dsn, "product", pid("pg")).unwrap();
    assert!(!a.can_write());
    assert_eq!(a.list_tables().unwrap(), ["event_log"], "login_accounts is not granted, so it is not even visible");
    assert!(a.schema_check().unwrap().ok());
    let b1 = a.read_events(&Watermark::Sequence(0), 3).unwrap();
    assert_eq!(b1.events.iter().map(|e| e.sequence.unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(b1.more && b1.next == Watermark::Sequence(3));
    let b2 = a.read_events(&b1.next, 100).unwrap();
    assert_eq!(b2.events.len(), 4);
    assert!(a.read_events(&b2.next, 100).unwrap().events.is_empty());
    assert_eq!(b1.events[0].event_time, "2026-10-01 10:01:00+00");
    let dump = format!("{b1:?}{b2:?}");
    assert!(!dump.contains("SECRET"));
    assert!(a.read_events(&Watermark::Sequence(0), 0).is_err());
    assert!(a.read_dimension("login_accounts", 5).is_err());
    // the role itself has no grant on payload or on the credential table
    let mut c: Client = db.ro_dsn.parse::<Config>().unwrap().connect(NoTls).unwrap();
    assert!(c.query("SELECT payload FROM product.event_log", &[]).is_err());
    assert!(c.query("SELECT password_hash FROM product.login_accounts", &[]).is_err());
}

#[test]
fn session_is_forced_read_only_even_for_a_role_that_is_not() {
    let Some(db) = Db::create() else { return };
    // the app role is not read-only by default: the adapter must refuse to run on it
    let r = PostgresProduct::connect(&db.app_dsn, "product", pid("pg"));
    assert!(r.is_ok(), "session options force read-only even for a role without the default: {:?}", r.err());
    let a = r.unwrap();
    assert!(!a.can_write());
    assert!(PostgresProduct::connect(&db.ro_dsn, "product; drop table x", pid("pg")).is_err());
}

#[test]
fn dataset_pg_maps_e0_to_platform_events_without_text_and_resumes_inside_a_batch() {
    let Some(db) = Db::create() else { return };
    let id = SourceId::new(DataMode::Dataset, "dataset:e0:test").unwrap();
    let a = DatasetPg::connect(&db.ro_dsn, "raw", id).unwrap();
    assert!(a.schema_check().unwrap().ok());
    let mut w = Watermark::origin_for("dataset-pg");
    let mut seen = vec![];
    loop {
        let b = a.read_events(&w, 2).unwrap();
        if b.events.is_empty() {
            break;
        }
        assert!(b.events.len() <= 2);
        seen.extend(b.events.iter().map(|e| (e.event_type.clone(), e.case_id.clone().unwrap())));
        w = b.next;
    }
    assert_eq!(seen.len(), 8, "3 opened + 3 turns + 2 closed, each exactly once: {seen:?}");
    assert_eq!(seen.iter().filter(|(t, _)| t == "case.opened").count(), 3);
    assert_eq!(seen.iter().filter(|(t, _)| t == "turn.created").count(), 3);
    assert_eq!(seen.iter().filter(|(t, _)| t == "case.closed").count(), 2);
    assert!(a.read_events(&Watermark::Sequence(0), 5).is_err());
    assert!(a.read_dimension("e0_case", 5).is_err());
    assert!(matches!(w, Watermark::Dataset { .. }));
}

#[test]
fn pg_store_meets_the_watermark_contract() {
    let Some(db) = Db::create() else { return };
    let s = PgStore::connect(&db.app_dsn).unwrap();
    let rec = |n: i64| WatermarkRecord { watermark: Watermark::Sequence(n), adapter: "product-postgres".into(), first_event_time: Some("2026-10-01".into()), cases_opened: 1, batches: 1 };
    let (a, b) = (pid("a"), pid("b"));
    assert_eq!(s.get(&a).unwrap(), None);
    s.commit(&a, None, &rec(5)).unwrap();
    s.commit(&b, None, &rec(1)).unwrap();
    s.commit(&a, Some(&Watermark::Sequence(5)), &rec(9)).unwrap();
    assert_eq!(s.get(&b).unwrap().unwrap().watermark, Watermark::Sequence(1));
    assert!(matches!(s.commit(&a, Some(&Watermark::Sequence(5)), &rec(12)), Err(SourceError::Conflict(_))));
    assert!(matches!(s.commit(&a, Some(&Watermark::Sequence(9)), &rec(3)), Err(SourceError::BadWatermark(_))));
    let d = SourceId::new(DataMode::Dataset, "dataset:e0:x").unwrap();
    assert!(matches!(s.commit(&d, None, &rec(1)), Err(SourceError::BadWatermark(_))));
    // a reader role cannot write the watermark table
    assert!(PgStore::connect(&db.ro_dsn).unwrap().commit(&pid("c"), None, &rec(1)).is_err());
}

#[test]
fn tick_runs_end_to_end_on_postgres_product_and_postgres_watermark() {
    let Some(db) = Db::create() else { return };
    let work = common::temp_path("pgtick").join("work");
    let c = MonCfg::from_pairs(&[("data_mode", "platform"), ("adapter", "product-postgres"), ("source_id", "platform:pg"), ("work_dir", work.to_str().unwrap()), ("runner_exe", env!("CARGO_BIN_EXE_sources-synth-runner")), ("batch_cap", "5")]).unwrap();
    let a = PostgresProduct::connect(&db.ro_dsn, "product", pid("pg")).unwrap();
    let store = PgStore::connect(&db.app_dsn).unwrap();
    let mut sizes = vec![];
    while let TickOutcome::Processed { events, record, .. } = tick(&c, &a, &store).unwrap() {
        assert_eq!(record["adapter"], "product-postgres");
        sizes.push(events);
    }
    assert_eq!(sizes, [5, 2]);
    assert_eq!(store.get(&pid("pg")).unwrap().unwrap().watermark, Watermark::Sequence(7));
}

#[test]
fn bad_dsn_and_identifiers_are_refused_before_any_connection() {
    assert!(matches!(PostgresProduct::connect("host=nowhere", "product; drop table x", pid("x")), Err(SourceError::AccessDenied(_))));
    assert!(matches!(PostgresProduct::connect("%% not a dsn", "product", pid("x")), Err(SourceError::BadConfig(_))));
    let d = SourceId::new(DataMode::Dataset, "dataset:e0:x").unwrap();
    assert!(matches!(DatasetPg::connect("host=nowhere", "public", d), Err(SourceError::BadConfig(_))));
}
