//! pg_engine_run: `engine_run demo3` over the Postgres store, for the kill -9 resume test.
//!   pg_engine_run demo3 <job_ref> [marker_file kill_after_index]
//! Env: PULSO_TEST_PG_ADMIN (server DSN), PULSO_TEST_PG_DB (database name), ENGINE_NOW, ENGINE_WORKER.
//! Prints the committed events then `SUMMARY {json}`. Exit 0 completed, 1 not completed, 2 usage.
use engine::executor::{execute, read_lease, ExecOptions};
use engine::{demo, event_log};
use pg::pgstore::PgJobStore;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 || a[0] != "demo3" {
        eprintln!("usage: pg_engine_run demo3 <job_ref> [marker idx]");
        std::process::exit(2);
    }
    let mut cfg: postgres::Config = std::env::var("PULSO_TEST_PG_ADMIN").expect("PULSO_TEST_PG_ADMIN").parse().expect("dsn");
    cfg.dbname(&std::env::var("PULSO_TEST_PG_DB").expect("PULSO_TEST_PG_DB"));
    let store = PgJobStore::open(&cfg, "tenant-a", &a[1]).expect("store");
    let now: u64 = std::env::var("ENGINE_NOW").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
    });
    let worker = std::env::var("ENGINE_WORKER").unwrap_or_else(|_| "w1".into());
    let mut o = ExecOptions::new("job-1", &worker, now);
    if let (Some(marker), Some(n)) = (a.get(2).cloned(), a.get(3).and_then(|v| v.parse::<usize>().ok())) {
        o.after_commit = Some(Box::new(move |i| {
            if i == n {
                std::fs::write(&marker, "x").unwrap();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }));
    }
    let handlers = demo::handlers();
    let result = execute(&store, &handlers, "x", &o);
    let log = event_log(&store, handlers.len()).unwrap_or_default();
    for l in &log {
        println!("{l}");
    }
    let (fence, attempt) = read_lease(&store).ok().flatten().map_or((0, 0), |l| (l.fence_token, l.attempt));
    println!("SUMMARY {{\"job\":\"{}\",\"events\":{},\"fence\":{},\"attempt\":{},\"ok\":{}}}", a[1], log.len(), fence, attempt, result.is_ok());
    if let Err(e) = &result {
        eprintln!("pg_engine_run: {e:?}");
        std::process::exit(1);
    }
}
