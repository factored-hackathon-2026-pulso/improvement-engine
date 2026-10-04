//! Embedded migrations, DSN handling, and the advisory-lock guarantee that two tasks do not both migrate.
//! The Postgres test is skipped unless PULSO_TEST_PG_ADMIN is set (a server DSN allowed to CREATE DATABASE).
use postgres::{Client, Config, NoTls};
use pulso::config::Secret;
use pulso::run::db;
use std::path::PathBuf;

fn repo_migrations() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../migrations")
}

#[test]
fn embedded_migrations_are_exactly_the_repo_files() {
    let embedded = db::migrations().unwrap();
    let on_disk = pg::migrate::discover(&repo_migrations()).unwrap();
    let key = |m: &pg::migrate::Migration| (m.id.clone(), m.checksum.clone());
    assert_eq!(embedded.iter().map(key).collect::<Vec<_>>(), on_disk.iter().map(key).collect::<Vec<_>>());
    assert!(embedded.iter().any(|m| m.id == "0051_pulso_job_claim_commit"));
}

#[test]
fn a_bad_dsn_is_refused_without_echoing_it() {
    let e = db::pg_config(&Secret::new("postgres://user:sup3rsecret@host:notaport/db")).unwrap_err();
    assert!(!e.contains("sup3rsecret") && !e.contains("host"), "{e}");
    assert!(e.contains("PULSO_DATABASE_URL"));
}

#[test]
fn two_supervisors_on_one_database_do_not_both_migrate() {
    let Ok(admin_dsn) = std::env::var("PULSO_TEST_PG_ADMIN") else {
        eprintln!("SKIP: PULSO_TEST_PG_ADMIN not set");
        return;
    };
    let admin: Config = admin_dsn.parse().expect("admin dsn");
    let name = format!("r1r_{}", std::process::id());
    admin.connect(NoTls).unwrap().batch_execute(&format!("CREATE DATABASE {name}")).unwrap();
    let mut cfg = admin.clone();
    cfg.dbname(&name);
    let total = db::migrations().unwrap().len();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let cfg = cfg.clone();
            std::thread::spawn(move || db::apply(&cfg))
        })
        .collect();
    let reports: Vec<_> = handles.into_iter().map(|h| h.join().unwrap().unwrap()).collect();
    let applied: Vec<usize> = reports.iter().map(|r| r.applied.len()).collect();
    let mut c: Client = cfg.connect(NoTls).unwrap();
    let rows: i64 = c.query_one("SELECT count(*) FROM pulso_schema_migrations", &[]).unwrap().get(0);
    let kv: bool = c.query_one("SELECT to_regclass('pulso_job_kv') IS NOT NULL", &[]).unwrap().get(0);
    let probe = db::PgProbe::new(cfg.clone());
    let ping = pulso::health::DbProbe::ping(&probe);
    drop(c);
    let _ = admin.connect(NoTls).unwrap().batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"));
    assert_eq!(applied.iter().sum::<usize>(), total, "each migration applied exactly once across both: {applied:?}");
    assert!(applied.contains(&0), "the loser applied nothing: {applied:?}");
    assert_eq!(rows as usize, total);
    assert!(kv);
    assert_eq!(ping, Ok(()));
}
