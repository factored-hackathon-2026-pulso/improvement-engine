mod common;
use common::{migrations_dir, TempDb};
use pg::migrate::{self, Error};

#[test]
fn discover_orders_by_version_then_name_and_tolerates_gaps() {
    let ids: Vec<String> = migrate::discover(&migrations_dir()).unwrap().into_iter().map(|m| m.id).collect();
    let pos = |s: &str| ids.iter().position(|i| i == s).unwrap_or_else(|| panic!("missing {s}"));
    assert!(pos("0001_pulso_artifact_revisions") < pos("0002_model_attempt_ledger"));
    assert!(pos("0002_model_attempt_ledger") < pos("0002_pulso_memory_control"));
    assert!(pos("0002_pulso_memory_control") < pos("0002_pulso_platform_observations"));
    assert!(pos("0002_pulso_platform_observations") < pos("0003_pulso_run_events"));
    assert!(pos("0003_pulso_run_events") < pos("0004_pulso_memory_temporal_receipts"));
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "file-name order is the apply order");
}

#[test]
fn parse_name_rules() {
    assert_eq!(migrate::parse_name("0050_widen_job_status.sql").unwrap(), (50, "0050_widen_job_status".into()));
    for bad in ["50_x.sql", "0050.sql", "0050_X.sql", "0050_x.txt", "abcd_x.sql"] {
        assert!(matches!(migrate::parse_name(bad), Err(Error::BadName(_))), "{bad}");
    }
}

#[test]
fn checksum_ignores_crlf() {
    assert_eq!(migrate::checksum("a\r\nb\n"), migrate::checksum("a\nb\n"));
    assert_eq!(migrate::checksum("x").len(), 64);
}

fn mk(id: &str, version: u32, sql: &str) -> migrate::Migration {
    migrate::Migration { id: id.into(), version, checksum: migrate::checksum(sql), sql: sql.into() }
}

#[test]
fn runner_is_idempotent_and_records_checksums() {
    let Some(db) = TempDb::create() else { return };
    let mut c = db.connect();
    let all = migrate::discover(&migrations_dir()).unwrap();
    let first = migrate::migrate(&mut c, &all).unwrap();
    assert_eq!(first.applied.len(), all.len());
    let second = migrate::migrate(&mut c, &all).unwrap();
    assert!(second.applied.is_empty() && second.skipped.len() == all.len());
    let rows = c.query("SELECT id, checksum FROM pulso_schema_migrations ORDER BY id", &[]).unwrap();
    assert_eq!(rows.len(), all.len());
    for (r, m) in rows.iter().zip(&all) {
        assert_eq!((r.get::<_, String>(0), r.get::<_, String>(1)), (m.id.clone(), m.checksum.clone()));
    }
}

#[test]
fn edited_applied_migration_is_rejected() {
    let Some(db) = TempDb::create() else { return };
    let mut c = db.connect();
    migrate::migrate(&mut c, &[mk("0900_t", 900, "CREATE TABLE t_a (x int)")]).unwrap();
    let err = migrate::migrate(&mut c, &[mk("0900_t", 900, "CREATE TABLE t_a (x bigint)")]).unwrap_err();
    assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err:?}");
}

#[test]
fn failed_migration_rolls_back_and_is_not_recorded() {
    let Some(db) = TempDb::create() else { return };
    let mut c = db.connect();
    let bad = mk("0901_bad", 901, "CREATE TABLE t_b (x int); SELECT 1/0");
    assert!(matches!(migrate::migrate(&mut c, &[bad]), Err(Error::Db(_))));
    let n: i64 = c.query_one("SELECT count(*) FROM pg_tables WHERE tablename = 't_b'", &[]).unwrap().get(0);
    let m: i64 = c.query_one("SELECT count(*) FROM pulso_schema_migrations", &[]).unwrap().get(0);
    assert_eq!((n, m), (0, 0));
}

#[test]
fn discover_executes_lf_normalised_sql_so_crlf_checkouts_build_the_same_schema() {
    let dir = std::env::temp_dir().join(format!("mig0_crlf_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("0001_a.sql"), "CREATE FUNCTION f() RETURNS int AS $$\r\nSELECT 1\r\n$$ LANGUAGE sql;\r\n").unwrap();
    let m = migrate::discover(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!m[0].sql.contains('\r'), "executed SQL must be LF-normalised");
}

#[test]
fn concurrent_runners_apply_each_migration_exactly_once() {
    let Some(db) = TempDb::create() else { return };
    let all = migrate::discover(&migrations_dir()).unwrap();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let mut c = db.connect();
            let all = all.clone();
            std::thread::spawn(move || migrate::migrate(&mut c, &all).unwrap())
        })
        .collect();
    let applied: usize = handles.into_iter().map(|h| h.join().unwrap().applied.len()).sum();
    assert_eq!(applied, all.len());
}
