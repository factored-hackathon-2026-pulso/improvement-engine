//! Served behind a reverse proxy under a path prefix, and the container healthcheck.
use debug_api::Store;
use pulso::config::RunConfig;
use pulso::health::{DbProbe, Health, Migrations};
use pulso::healthcheck;
use pulso::run::http::HttpTask;
use pulso::run::log::Logger;
use pulso::run::supervisor::{StopToken, Supervisor};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

struct Up;
impl DbProbe for Up {
    fn ping(&self) -> Result<(), String> {
        Ok(())
    }
}

fn get(addr: SocketAddr, path: &str, bearer: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let auth = bearer.map(|b| format!("Authorization: Bearer {b}\r\n")).unwrap_or_default();
    write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    (raw.split(' ').nth(1).unwrap().parse().unwrap(), raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}

struct Srv {
    addr: SocketAddr,
    health: Arc<Health>,
    stop: StopToken,
    join: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Srv {
    fn drop(&mut self) {
        self.stop.stop();
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn start(base: &str, console: Option<&std::path::Path>) -> Srv {
    let mut m: HashMap<String, String> = [("PULSO_STORAGE", "memory"), ("PULSO_DATA_MODE", "dataset"), ("PULSO_LISTEN_ADDR", "127.0.0.1:0"), ("PULSO_DEBUG_TOKEN", TOKEN)].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    m.insert("PULSO_BASE_PATH".into(), base.into());
    if let Some(c) = console {
        m.insert("PULSO_CONSOLE_DIR".into(), c.display().to_string());
    }
    let cfg = RunConfig::from_lookup(&|k| m.get(k).cloned()).unwrap();
    let health = Health::new(Arc::new(Up));
    health.set_migrations(Migrations::Applied);
    let task = HttpTask::bind(&cfg, health.clone(), Arc::new(Store::memory())).unwrap();
    let addr = task.local_addr();
    let sink: Box<dyn Write + Send> = Box::new(std::io::sink());
    let mut sup = Supervisor::new(health.clone(), Logger::new(sink), Duration::from_secs(2));
    sup.add(Box::new(task));
    let stop = sup.stop_token();
    let join = Some(std::thread::spawn(move || {
        sup.run();
    }));
    Srv { addr, health, stop, join }
}

fn console_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pulso-prefix-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("index.html"), "<html>console</html>").unwrap();
    std::fs::write(d.join("config.json"), "{\"apiBase\":\"\"}").unwrap();
    d
}

#[test]
fn routes_and_static_files_resolve_under_the_prefix() {
    let dir = console_dir("a");
    let s = start("/pulso", Some(&dir));
    assert_eq!(get(s.addr, "/pulso/internal/v1/debug/runs", Some(TOKEN)).0, 200);
    assert_eq!(get(s.addr, "/pulso/internal/v1/debug/runs", None).0, 401, "auth still applies under the prefix");
    let (st, body) = get(s.addr, "/pulso/", None);
    assert_eq!((st, body.contains("console")), (200, true));
    assert_eq!(get(s.addr, "/pulso", None).0, 200);
    assert_eq!(get(s.addr, "/pulso/index.html", None).0, 200);
    let (st, cfg) = get(s.addr, "/pulso/config.json", None);
    assert_eq!(st, 200);
    assert!(cfg.contains("\"apiBase\":\"/pulso\""), "the console must call the API under the prefix: {cfg}");
    assert_eq!(get(s.addr, "/pulso/healthz", None).0, 200);
    assert_eq!(get(s.addr, "/pulso/readyz", None).0, 200);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn with_a_prefix_nothing_but_probes_is_reachable_outside_it() {
    let dir = console_dir("b");
    let s = start("/pulso", Some(&dir));
    for p in ["/internal/v1/debug/runs", "/index.html", "/config.json", "/", "/pulsox/internal/v1/debug/runs", "/pulso/../internal/v1/debug/runs"] {
        let (st, _) = get(s.addr, p, Some(TOKEN));
        assert_eq!(st, 404, "{p} must not be exposed outside the prefix");
    }
    assert_eq!(get(s.addr, "/healthz", None).0, 200, "probes stay reachable without the prefix");
    assert_eq!(get(s.addr, "/readyz", None).0, 200);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn without_a_prefix_nothing_changes() {
    let s = start("", None);
    assert_eq!(get(s.addr, "/internal/v1/debug/runs", Some(TOKEN)).0, 200);
    assert_eq!(get(s.addr, "/pulso/internal/v1/debug/runs", Some(TOKEN)).0, 404);
}

#[test]
fn healthcheck_is_ok_only_on_a_ready_200() {
    let s = start("", None);
    let port = s.addr.port();
    assert_eq!(healthcheck::check(port, "", Duration::from_secs(2)), Ok(()));
    s.health.set_migrations(Migrations::Failed("x".into()));
    let e = healthcheck::check(port, "", Duration::from_secs(2)).unwrap_err();
    assert!(e.contains("503"), "{e}");
    s.health.set_migrations(Migrations::Applied);
    assert_eq!(healthcheck::main(&["--port".into(), port.to_string()]), 0);
    s.health.begin_shutdown();
    assert_eq!(healthcheck::main(&["--port".into(), port.to_string()]), 1);
}

#[test]
fn healthcheck_honours_the_prefix_and_fails_fast_when_nothing_listens() {
    let s = start("/pulso", None);
    assert_eq!(healthcheck::check(s.addr.port(), "/pulso", Duration::from_secs(2)), Ok(()));
    let free = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let t = std::time::Instant::now();
    let e = healthcheck::check(free, "", Duration::from_millis(500)).unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(3));
    assert!(!e.contains(TOKEN));
    assert_eq!(healthcheck::main(&["--port".into(), "notaport".into()]), 2);
}

// ---- adversarial review (CL): normalisation tricks around the prefix ----

/// Raw request line, so encodings reach the server exactly as written.
fn raw(addr: SocketAddr, target: &str, bearer: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let auth = bearer.map(|b| format!("Authorization: Bearer {b}\r\n")).unwrap_or_default();
    let _ = write!(s, "GET {target} HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n");
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let raw = String::from_utf8_lossy(&buf).to_string();
    (raw.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0), raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}

#[test]
fn prefix_tricks_never_reach_the_debug_api_without_the_token() {
    let dir = console_dir("c");
    let outside = dir.parent().unwrap().join("pulso-secret-outside.txt");
    std::fs::write(&outside, "outside").unwrap();
    let s = start("/pulso", Some(&dir));
    let api = "internal/v1/debug/runs";
    let tricks = [
        format!("//{api}"),
        format!("/pulso//{api}"),
        format!("/pulso/./{api}"),
        format!("/pulso/%2e%2e/{api}"),
        format!("/pulso/%2E%2E/{api}"),
        format!("/pulso/..%2f{api}"),
        format!("/pulso/%2e%2e%2f{api}"),
        format!("/pulso/%252e%252e/{api}"),
        format!("/pulso/..\\{api}"),
        format!("/pulso\\..\\{api}"),
        format!("/pulso%2f{api}"),
        format!("/PULSO/{api}"),
        format!("/pulso/{api}%00"),
        format!("/pulso/{}", api.to_uppercase()),
        format!("http://evil.example/{api}"),
        format!("http://evil.example/pulso/{api}"),
        "/pulso/..%5c..%5cpulso-secret-outside.txt".to_string(),
        "/pulso/....//pulso-secret-outside.txt".to_string(),
        "/pulso/%2e%2e/pulso-secret-outside.txt".to_string(),
    ];
    for t in &tricks {
        let (st, body) = raw(s.addr, t, None);
        // 200 is only ever the public static console shell (SPA fallback), never API data
        assert!(st != 200 || body == "<html>console</html>", "{t} answered {st} without a token: {body}");
        assert!(!body.contains("outside") && !body.contains("\"items\""), "{t} leaked: {body}");
    }
    for t in &tricks {
        let (_, body) = raw(s.addr, t, Some(TOKEN));
        assert!(!body.contains("outside"), "{t} leaked a file outside the console dir: {body}");
    }
    let _ = std::fs::remove_file(outside);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn oversized_request_targets_are_refused_and_the_front_stays_up() {
    let s = start("/pulso", None);
    let long = "a".repeat(100_000);
    let (st, _) = raw(s.addr, &format!("/pulso/{long}"), None);
    assert_ne!(st, 200);
    assert_eq!(get(s.addr, "/readyz", None).0, 200);
}
