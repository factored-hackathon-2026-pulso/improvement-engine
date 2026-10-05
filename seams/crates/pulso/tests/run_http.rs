//! The `pulso run` HTTP surface: /healthz and /readyz in front of the embedded debug-api, bearer auth, bounded stop.
use debug_api::Store;
use pulso::config::RunConfig;
use pulso::health::{DbProbe, Health, Migrations};
use pulso::run::http::HttpTask;
use pulso::run::log::Logger;
use pulso::run::supervisor::Supervisor;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

struct Up;
impl DbProbe for Up {
    fn ping(&self) -> Result<(), String> {
        Ok(())
    }
}

fn cfg(extra: &[(&str, &str)]) -> RunConfig {
    let mut m: HashMap<String, String> = [("PULSO_STORAGE", "memory"), ("PULSO_DATA_MODE", "dataset"), ("PULSO_LISTEN_ADDR", "127.0.0.1:0")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    m.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    RunConfig::from_lookup(&|k| m.get(k).cloned()).unwrap()
}

fn get(addr: SocketAddr, path: &str, bearer: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let auth = bearer.map(|b| format!("Authorization: Bearer {b}\r\n")).unwrap_or_default();
    write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let status = raw.split(' ').nth(1).unwrap().parse().unwrap();
    (status, raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}

struct Running {
    addr: SocketAddr,
    health: Arc<Health>,
    stop: pulso::run::supervisor::StopToken,
    join: std::thread::JoinHandle<pulso::run::supervisor::Outcome>,
}

fn start(c: &RunConfig) -> Running {
    let health = Health::new(Arc::new(Up));
    let task = HttpTask::bind(c, health.clone(), Arc::new(Store::memory())).unwrap();
    let addr = task.local_addr();
    let sink: Box<dyn Write + Send> = Box::new(std::io::sink());
    let mut sup = Supervisor::new(health.clone(), Logger::new(sink), Duration::from_secs(2));
    sup.add(Box::new(task));
    let stop = sup.stop_token();
    let join = std::thread::spawn(move || sup.run());
    Running { addr, health, stop, join }
}

#[test]
fn healthz_is_always_up_and_readyz_names_the_reason_then_turns_green() {
    let r = start(&cfg(&[]));
    assert_eq!(get(r.addr, "/healthz", None).0, 200);
    let (st, body) = get(r.addr, "/readyz", None);
    assert_eq!(st, 503);
    assert!(body.contains("migrations_pending"), "{body}");
    r.health.set_migrations(Migrations::Applied);
    let t = Instant::now();
    loop {
        let (st, body) = get(r.addr, "/readyz", None);
        if st == 200 {
            assert!(body.contains("\"ready\":true"), "{body}");
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(3), "never ready: {st} {body}");
        std::thread::sleep(Duration::from_millis(20));
    }
    r.stop.stop();
    assert_eq!(r.join.join().unwrap().exit_code(), 0);
}

#[test]
fn debug_routes_need_the_bearer_token_but_probes_do_not() {
    let r = start(&cfg(&[("PULSO_DEBUG_TOKEN", TOKEN)]));
    r.health.set_migrations(Migrations::Applied);
    let path = "/internal/v1/debug/runs";
    assert_eq!(get(r.addr, path, None).0, 401);
    assert_eq!(get(r.addr, path, Some("wrong")).0, 401);
    assert_eq!(get(r.addr, path, Some(TOKEN)).0, 200);
    assert_eq!(get(r.addr, "/healthz", None).0, 200);
    assert_ne!(get(r.addr, "/readyz", None).0, 401);
    r.stop.stop();
    r.join.join().unwrap();
}

#[test]
fn a_non_loopback_bind_is_served_only_when_the_config_allowed_it_and_stays_authenticated() {
    let c = cfg(&[("PULSO_LISTEN_ADDR", "0.0.0.0:0"), ("PULSO_ALLOW_NON_LOOPBACK", "1"), ("PULSO_DEBUG_TOKEN", TOKEN)]);
    let r = start(&c);
    let probe = SocketAddr::from(([127, 0, 0, 1], r.addr.port()));
    assert!(r.addr.ip().is_unspecified());
    assert_eq!(get(probe, "/internal/v1/debug/runs", None).0, 401);
    assert_eq!(get(probe, "/internal/v1/debug/runs", Some(TOKEN)).0, 200);
    r.stop.stop();
    r.join.join().unwrap();
}

#[test]
fn stop_releases_the_port_within_the_grace_period() {
    let r = start(&cfg(&[]));
    let addr = r.addr;
    assert_eq!(get(addr, "/healthz", None).0, 200);
    let t = Instant::now();
    r.stop.stop();
    let out = r.join.join().unwrap();
    assert_eq!(out.exit_code(), 0, "{out:?}");
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err(), "listener must be closed");
}
