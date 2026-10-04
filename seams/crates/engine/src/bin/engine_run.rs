//! engine_run: run-once shell over the E2 executor (std only).
//!   engine_run demo3  <store_dir> [marker_file kill_after_index]
//!   engine_run thread <store_dir> <work_dir> [marker_file kill_after_index] [--live-stubs]
//! Prints the committed event sequence (one event per line), then one line `SUMMARY {json}`.
//! Env: ENGINE_NOW (unix seconds; default system clock) so a test can step the lease clock;
//! ENGINE_WORKER (default `w1`). With a marker, the file is created after handler N commits and the
//! process then blocks, waiting to be killed. Exit: 0 completed, 1 job not completed (reason on stderr), 2 usage.
use abi::JobHandler;
use engine::executor::{execute, read_lease, ExecError, ExecOptions};
use engine::{demo, event_log, FileStore};

fn now() -> u64 {
    match std::env::var("ENGINE_NOW").ok().and_then(|v| v.parse().ok()) {
        Some(n) => n,
        None => std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
    }
}

fn marker_hook(args: &[String]) -> Option<Box<dyn Fn(usize)>> {
    let (marker, n) = (args.first()?.clone(), args.get(1)?.parse::<usize>().ok()?);
    Some(Box::new(move |i| {
        if i == n {
            std::fs::write(&marker, "x").unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }))
}

fn run(job: &str, store: &FileStore, handlers: &[Box<dyn JobHandler>], initial: &str, hook: Option<Box<dyn Fn(usize)>>) -> i32 {
    let worker = std::env::var("ENGINE_WORKER").unwrap_or_else(|_| "w1".into());
    let mut o = ExecOptions::new(job, &worker, 0);
    let t = now();
    o.now = Box::new(move || t);
    o.after_commit = hook;
    let result = execute(store, handlers, initial, &o);
    let log = event_log(store, handlers.len()).unwrap_or_default();
    for l in &log {
        println!("{l}");
    }
    let (fence, attempt) = read_lease(store).ok().flatten().map_or((0, 0), |l| (l.fence_token, l.attempt));
    let (status, detail) = match &result {
        Ok(_) => ("completed", String::new()),
        Err(ExecError::LeaseHeld { expires_at }) => ("lease_held", format!("expires_at {expires_at}")),
        Err(ExecError::NeedsReconciliation(i)) => ("needs_reconciliation", format!("handler {i}")),
        Err(e) => ("failed", format!("{e:?}")),
    };
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let final_payload = result.as_ref().map_or(String::new(), |p| p.clone());
    println!(
        "SUMMARY {{\"job\":\"{}\",\"status\":\"{}\",\"events\":{},\"fence\":{},\"attempt\":{},\"detail\":\"{}\",\"final_bytes\":{}}}",
        esc(job),
        status,
        log.len(),
        fence,
        attempt,
        esc(&detail),
        final_payload.len()
    );
    if result.is_ok() {
        0
    } else {
        eprintln!("engine_run: {status} {detail}");
        1
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let code = match a.first().map(String::as_str) {
        Some("demo3") if a.len() >= 2 => {
            let store = FileStore::open(&a[1]).expect("store");
            run("job-1", &store, &demo::handlers(), "x", marker_hook(&a[2..]))
        }
        _ => {
            eprintln!("usage: engine_run demo3 <store> [marker idx] | thread <store> <work_dir> [marker idx] [--live-stubs]");
            2
        }
    };
    std::process::exit(code);
}
