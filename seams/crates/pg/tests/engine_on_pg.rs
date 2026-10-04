//! The engine's executor suite and the kill -9 resume run against the Postgres-backed JobStore.
mod common;
use common::{migrations_dir, TempDb};
use engine::conformance::{run_suite, Backend, GOLDEN};
use engine::JobStore;
use pg::pgstore::PgJobStore;
use postgres::Config;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

struct Pg {
    cfg: Config,
    refs: Mutex<HashMap<String, String>>,
    n: std::sync::atomic::AtomicU32,
}

impl Backend for Pg {
    fn fresh(&self, name: &str) -> Box<dyn JobStore> {
        let r = format!("{name}-{}", self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        self.refs.lock().unwrap().insert(name.into(), r.clone());
        Box::new(PgJobStore::open(&self.cfg, "tenant-a", &r).unwrap())
    }
    fn reopener(&self, name: &str) -> Arc<dyn Fn() -> Box<dyn JobStore>> {
        let r = self.refs.lock().unwrap()[name].clone();
        let cfg = self.cfg.clone();
        Arc::new(move || Box::new(PgJobStore::open(&cfg, "tenant-a", &r).unwrap()))
    }
}

fn migrated() -> Option<TempDb> {
    let db = TempDb::create()?;
    let mut c = db.connect();
    pg::migrate::migrate(&mut c, &pg::migrate::discover(&migrations_dir()).unwrap()).unwrap();
    Some(db)
}

#[test]
fn executor_suite_on_the_pg_store() {
    let Some(db) = migrated() else { return };
    let b = Pg { cfg: db.config(), refs: Mutex::new(HashMap::new()), n: Default::default() };
    let f = run_suite(&b);
    assert!(f.is_empty(), "{f:#?}");
}

#[test]
fn kill_9_between_handlers_resumes_on_pg_with_the_golden_sequence() {
    use std::process::{Command, Stdio};
    let Some(db) = migrated() else { return };
    let exe = env!("CARGO_BIN_EXE_pg_engine_run");
    let run = |job: &str, now: &str| {
        let mut c = Command::new(exe);
        c.args(["demo3", job]).env("ENGINE_NOW", now).env("PULSO_TEST_PG_DB", &db.name);
        c
    };
    let events = |out: &[u8]| -> String {
        String::from_utf8_lossy(out).lines().filter(|l| !l.starts_with("SUMMARY ")).map(|l| format!("{l}\n")).collect()
    };
    let clean = run("clean", "1000").output().unwrap();
    assert!(clean.status.success(), "{}", String::from_utf8_lossy(&clean.stderr));
    assert_eq!(events(&clean.stdout), GOLDEN);

    let marker = std::env::temp_dir().join(format!("pgkill-{}.marker", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let mut child = run("killed", "1000").arg(&marker).arg("1").stdout(Stdio::null()).spawn().unwrap();
    for _ in 0..500 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists(), "handler 1 never committed");
    child.kill().unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_file(&marker);
    // lease (60 s from 1000) still live: restart refused; at expiry reclaimed with attempt 2, identical log
    let early = run("killed", "1059").output().unwrap();
    assert!(!early.status.success());
    let resumed = run("killed", "1060").output().unwrap();
    assert!(resumed.status.success(), "{}", String::from_utf8_lossy(&resumed.stderr));
    assert_eq!(events(&resumed.stdout), GOLDEN);
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("\"attempt\":2"));
}
