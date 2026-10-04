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

#[test]
fn shared_repo_instances_over_one_database_see_the_same_jobs() {
    let Ok(admin_dsn) = std::env::var("PULSO_TEST_PG_ADMIN") else {
        eprintln!("SKIP: PULSO_TEST_PG_ADMIN not set");
        return;
    };
    let admin: Config = admin_dsn.parse().expect("admin dsn");
    let name = format!("r1r_shared_{}", std::process::id());
    admin.connect(NoTls).unwrap().batch_execute(&format!("CREATE DATABASE {name}")).unwrap();
    let mut cfg = admin.clone();
    cfg.dbname(&name);
    db::apply(&cfg).unwrap();
    use pg::repo::JobRepository;
    let (a, b) = (pg::pgrepo::PgRepo::shared(cfg.clone()), pg::pgrepo::PgRepo::shared(cfg.clone()));
    let id = a.admit("tenant-x").unwrap();
    let claimed = b.claim_next("tenant-x", "w2", 1_000, 10).unwrap();
    let _ = admin.connect(NoTls).unwrap().batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"));
    assert_eq!(claimed.map(|c| c.job), Some(id), "a second process must claim what the first admitted");
}

// ---- adversarial review (CL) ----

fn scratch_db(tag: &str) -> Option<(Config, Config, String)> {
    let admin_dsn = std::env::var("PULSO_TEST_PG_ADMIN").ok()?;
    let admin: Config = admin_dsn.parse().expect("admin dsn");
    let name = format!("r1r_{tag}_{}", std::process::id());
    admin.connect(NoTls).unwrap().batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)")).unwrap();
    admin.connect(NoTls).unwrap().batch_execute(&format!("CREATE DATABASE {name}")).unwrap();
    let mut cfg = admin.clone();
    cfg.dbname(&name);
    Some((admin, cfg, name))
}

fn drop_db(admin: &Config, name: &str) {
    let _ = admin.connect(NoTls).unwrap().batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"));
}

#[test]
fn a_migration_edited_after_it_was_applied_is_refused_by_id_and_a_rerun_is_otherwise_a_no_op() {
    let Some((admin, cfg, name)) = scratch_db("drift") else { return eprintln!("SKIP: PULSO_TEST_PG_ADMIN not set") };
    let first = db::apply(&cfg).unwrap();
    assert!(first.applied.len() > 5);
    assert_eq!(db::apply(&cfg).unwrap().applied.len(), 0, "second run applies nothing");
    // simulate drift: the recorded checksum of one migration no longer matches the embedded file
    let mut c: Client = cfg.connect(NoTls).unwrap();
    c.batch_execute(&format!("UPDATE pulso_schema_migrations SET checksum = repeat('0', 64) WHERE id = '0051_pulso_job_claim_commit'")).unwrap();
    drop(c);
    let e = db::apply(&cfg).unwrap_err();
    drop_db(&admin, &name);
    assert!(e.contains("0051_pulso_job_claim_commit") && e.contains("edited"), "{e}");
}

#[test]
fn a_connection_that_dies_holding_the_migration_lock_does_not_wedge_the_next_migrator() {
    let Some((admin, cfg, name)) = scratch_db("lock") else { return eprintln!("SKIP: PULSO_TEST_PG_ADMIN not set") };
    let mut holder: Client = cfg.connect(NoTls).unwrap();
    holder.execute("SELECT pg_advisory_lock($1)", &[&0x7075_6c73_6f5f_6d67_i64]).unwrap();
    let c2 = cfg.clone();
    let t = std::thread::spawn(move || db::apply(&c2));
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(!t.is_finished(), "the migrator waits while the lock is held");
    drop(holder); // process crash == connection closed: the session lock is released by the server
    let start = std::time::Instant::now();
    while !t.is_finished() && start.elapsed() < std::time::Duration::from_secs(120) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let finished = t.is_finished();
    let r = if finished { Some(t.join().unwrap()) } else { None };
    drop_db(&admin, &name);
    assert!(finished, "migrator still blocked 120s after the holder dropped");
    let r = r.unwrap();
    assert!(r.is_ok(), "migration failed after the lock was released: {r:?}");
}

#[test]
fn shared_repos_keep_tenants_apart_and_never_hand_one_tenants_job_to_another() {
    let Some((admin, cfg, name)) = scratch_db("tenants") else { return eprintln!("SKIP: PULSO_TEST_PG_ADMIN not set") };
    db::apply(&cfg).unwrap();
    use pg::repo::JobRepository;
    let (a, b) = (pg::pgrepo::PgRepo::shared(cfg.clone()), pg::pgrepo::PgRepo::shared(cfg.clone()));
    let ja = a.admit("tenant-a").unwrap();
    let jb = b.admit("tenant-b").unwrap();
    let got_b = b.claim_next("tenant-b", "w", 1_000, 10).unwrap().map(|c| c.job);
    let got_a_other = a.claim_next("tenant-c", "w", 1_000, 10).unwrap();
    let got_a = a.claim_next("tenant-a", "w", 1_000, 10).unwrap().map(|c| c.job);
    // a holder of tenant-a's job cannot commit under tenant-b's name
    let fence = 1;
    let cross = b.commit_output("tenant-b", &ja, 0, "w", fence, 1_001, "x");
    drop_db(&admin, &name);
    assert_eq!((got_b, got_a), (Some(jb), Some(ja)));
    assert!(got_a_other.is_none(), "a tenant with no jobs claims nothing");
    assert!(cross.is_err(), "cross-tenant commit must fail");
}
