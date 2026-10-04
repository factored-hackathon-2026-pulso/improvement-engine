//! pulso serve | demo. Exit 0 ok; 1 the run or the server failed; 2 usage or an honest refusal.
//!
//! `serve`: the debug-api embedded (loopback only), admin append enabled for this local run, the built console served from
//! `--console-dir` (default debug-console/dist next to the repo when present) with `/config.json` pointing the console's
//! data provider (`http`, same origin) at this very server. Prints `pulso listening on http://<addr>` once bound.
//! `demo`: runs the ten-step offline thread (DEMO-0, offline DoublePort) and streams every step, double and gate into a
//! running `pulso serve` through the admin append route while it runs (`--pace-ms` slows it for a human), then prints the
//! doubles[] first and the steps with their labels.
use debug_api::server::{bind_loopback, serve};
use debug_api::{App, Config, Store};
use pulso::cli::{Cmd, DemoArgs, ServeArgs, USAGE, parse, real_core_refusal};
use pulso::doubles::{generate, summary};
use pulso::live::{DemoOpts, demo};
use pulso::sink::HttpSink;
use serde_json::json;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn die(code: i32, msg: &str) -> ! {
    eprintln!("pulso: {msg}");
    std::process::exit(code)
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn random_token() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{:032x}", nanos ^ (u128::from(std::process::id()) << 64))
}

fn default_console_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let cwd = std::env::current_dir().ok()?;
    exe.ancestors().chain(cwd.ancestors()).map(|a| a.join("debug-console").join("dist")).find(|d| d.join("index.html").is_file())
}

fn run_serve(a: ServeArgs) {
    let server = bind_loopback(&a.addr).unwrap_or_else(|e| die(2, &e));
    if a.exit_on_stdin_eof {
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            std::process::exit(0)
        });
    }
    let store = Arc::new(match &a.store_dir {
        Some(d) => Store::open(d).unwrap_or_else(|e| die(1, &e)),
        None => Store::memory(),
    });
    let admin = a.admin_token.or_else(|| env("PULSO_ADMIN_TOKEN")).unwrap_or_else(|| {
        let t = random_token();
        eprintln!("pulso: no admin token given; generated one for this run. `pulso demo` needs --admin-token or PULSO_ADMIN_TOKEN to append. Token: {t}");
        t
    });
    let console = a.console_dir.or_else(default_console_dir).filter(|d| d.join("index.html").is_file());
    match &console {
        Some(d) => eprintln!("pulso: serving the built console from {}", d.display()),
        None => eprintln!("pulso: no built console found (debug-console/dist/index.html); serving the API only. Build it: cd debug-console && npm ci && npm run build"),
    }
    let config_json = json!({"provider": "stand-in", "dataProvider": "http", "apiBase": "", "sseHeartbeatMs": 5000, "traceLinkOrigins": []}).to_string();
    let cfg = Config { token: a.token.or_else(|| env("PULSO_DEBUG_TOKEN")), admin_token: Some(admin), static_dir: console, config_json: Some(config_json), ..Config::default() };
    let bound = server.server_addr().to_ip().map_or(a.addr.clone(), |s| s.to_string());
    println!("pulso listening on http://{bound}");
    let _ = std::io::stdout().flush();
    serve(server, Arc::new(App::new(store, cfg)));
}

fn run_demo(a: DemoArgs) {
    if a.real_core {
        let url = env("PULSO_CORE_URL");
        let reachable = url.as_deref().is_some_and(|u| {
            let hostport = u.trim_start_matches("http://").trim_start_matches("https://").split('/').next().unwrap_or("");
            hostport.to_socket_addrs().ok().and_then(|mut s| s.next()).is_some_and(|s| TcpStream::connect_timeout(&s, Duration::from_secs(2)).is_ok())
        });
        die(2, &real_core_refusal(url.as_deref(), reachable));
    }
    let token = a.admin_token.or_else(|| env("PULSO_ADMIN_TOKEN")).unwrap_or_else(|| die(2, "demo needs --admin-token or PULSO_ADMIN_TOKEN (the token `pulso serve` was started with)"));
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let run_id = a.run_id.unwrap_or_else(|| format!("run-demo0-{secs}"));
    let work = a.work_dir.or_else(|| env("PULSO_WORK_DIR").map(PathBuf::from)).unwrap_or_else(|| std::env::temp_dir().join(format!("pulso-demo-{run_id}")));
    let runner = env("STEPS_RUNNER_EXE").map(PathBuf::from).unwrap_or_else(|| {
        let mut p = std::env::current_exe().unwrap_or_else(|e| die(1, &e.to_string()));
        p.set_file_name(format!("pulso-synth-runner{}", std::env::consts::EXE_SUFFIX));
        p
    });
    if !runner.is_file() {
        die(1, &format!("sensor stand-in not found at {} (build it with `cargo build -p pulso`)", runner.display()));
    }
    let sink = HttpSink::new(&a.api, &token);
    if let Err(e) = TcpStream::connect_timeout(&a.api.to_socket_addrs().ok().and_then(|mut s| s.next()).unwrap_or_else(|| die(2, &format!("bad --api {:?}", a.api))), Duration::from_secs(3)) {
        die(1, &format!("cannot reach the debug-api at {} ({e}); start it with `pulso serve`", a.api));
    }
    let mut o = DemoOpts::new(run_id.clone(), work, runner);
    o.pace = Duration::from_millis(a.pace_ms);
    o.human_override = a.human_override;
    o.denied_kind = a.denied_kind;
    o.sha = a.sha.or_else(|| env("THREAD10_SHA")).filter(|s| s.len() == 40).unwrap_or(o.sha);
    eprintln!("pulso: streaming run {run_id} into http://{} (DEMO-0: offline Core double; every stand-in is labelled)", a.api);
    match demo(Arc::new(sink), &o) {
        Ok(run) => {
            let mut report = run.report;
            report["doubles"] = json!(generate(&report));
            println!("run: {run_id}  console: http://{}/#/run/{run_id}", a.api);
            print!("{}", summary(&report));
            if let Some(e) = run.error {
                eprintln!("pulso: the job stopped: {e}");
                std::process::exit(1);
            }
        }
        Err(e) => die(1, &e),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Ok(Cmd::Serve(a)) => run_serve(a),
        Ok(Cmd::Demo(a)) => run_demo(a),
        Err(e) => {
            eprintln!("pulso: {e}\n{USAGE}");
            std::process::exit(2)
        }
    }
}
