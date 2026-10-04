//! debug-api [--addr 127.0.0.1:4020] [--store-dir DIR] [--ingest REPORT.json]... [--seed-contract]
//!   [--automation-sim FILE] [--automation-e0 FILE] [--automation-proposals FILE]  (serves /internal/v1/automation; sim = SIM-ONLY draft stream)
//! Serves /internal/v1/debug for the debug-console. Loopback only. Env: DEBUG_API_TOKEN (static bearer for the debug
//! routes, default open), DEBUG_API_ADMIN_TOKEN (enables /__admin/v1/*: live append, purge, report ingest; default off),
//! DEBUG_API_HEARTBEAT_MS (SSE heartbeat, default 5000). Prints `debug-api listening on http://<addr>` once bound.
//! Exit 2: usage, bind or ingest error.
use debug_api::ingest::{contract_seed, engine_run_report};
use debug_api::server::{bind_loopback, serve};
use debug_api::{App, Config, Store};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

fn fail(msg: &str) -> ! {
    eprintln!("debug-api: {msg}");
    std::process::exit(2)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut auto: [Option<String>; 3] = [None, None, None];
    let (mut addr, mut dir, mut ingest, mut seed) = ("127.0.0.1:4020".to_string(), None, Vec::new(), false);
    while let Some(a) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| fail(&format!("{a} needs a value")));
        match a.as_str() {
            "--addr" => addr = value(),
            "--store-dir" => dir = Some(value()),
            "--ingest" => ingest.push(value()),
            "--seed-contract" => seed = true,
            "--automation-sim" => auto[0] = Some(value()),
            "--automation-e0" => auto[1] = Some(value()),
            "--automation-proposals" => auto[2] = Some(value()),
            _ => fail(&format!("unknown argument {a:?}")),
        }
    }
    let server = bind_loopback(&addr).unwrap_or_else(|e| fail(&e));
    let store = Arc::new(match &dir {
        Some(d) => Store::open(d).unwrap_or_else(|e| fail(&e)),
        None => Store::memory(),
    });
    for path in &ingest {
        let report = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()));
        let report = report.unwrap_or_else(|e| fail(&format!("{path}: {e}")));
        match engine_run_report(&*store, &|id| store.head(id).is_some(), &report) {
            Ok(run) => eprintln!("debug-api: ingested {path} as {run}"),
            Err(e) if e.contains("already") => eprintln!("debug-api: {e}; skipped"),
            Err(e) => fail(&format!("{path}: {e}")),
        }
    }
    if seed && store.head("run-active").is_none() {
        contract_seed(&*store).unwrap_or_else(|e| fail(&e));
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let heartbeat = env("DEBUG_API_HEARTBEAT_MS").and_then(|v| v.parse().ok()).unwrap_or(5000);
    let load = |p: &Option<String>| -> Option<serde_json::Value> {
        p.as_ref().map(|f| std::fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())).unwrap_or_else(|e| fail(&format!("{f}: {e}"))))
    };
    let automation = if auto.iter().any(Option::is_some) {
        Some(Arc::new(debug_api::automation::Automation::from_json(load(&auto[1]).as_ref(), load(&auto[0]).as_ref(), load(&auto[2]).as_ref()).unwrap_or_else(|e| fail(&e))))
    } else {
        None
    };
    let cfg = Config { token: env("DEBUG_API_TOKEN"), admin_token: env("DEBUG_API_ADMIN_TOKEN"), heartbeat: Duration::from_millis(heartbeat), automation, ..Config::default() };
    let bound = server.server_addr().to_ip().map_or(addr.clone(), |a| a.to_string());
    println!("debug-api listening on http://{bound}");
    let _ = std::io::stdout().flush();
    serve(server, Arc::new(App::new(store, cfg)));
}
