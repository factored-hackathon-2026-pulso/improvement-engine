//! Over a real socket: SSE framing (`id` = sequence), Last-Event-ID resume without loss or duplication, live append through
//! the sink, heartbeat comments, the 410 body, and the loopback-only bind.
use debug_api::server::{bind_loopback, serve};
use debug_api::{App, Config, NewEvent, RunEventSink, Store};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

const D: &str = "/internal/v1/debug";

fn boot(hb_ms: u64) -> (Arc<Store>, u16) {
    let store = Arc::new(Store::memory());
    let app = Arc::new(App::new(store.clone(), Config { heartbeat: Duration::from_millis(hb_ms), ..Config::default() }));
    let server = bind_loopback("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    std::thread::spawn(move || serve(server, app));
    (store, port)
}
fn ev(i: usize) -> NewEvent {
    NewEvent::new("node_status_changed", "node", "verify", json!({"node": {"node_id": "verify", "label": "verify", "stage": "verifier", "status": if i % 2 == 0 { "running" } else { "queued" }, "depends_on": [], "reason_code": null, "node_kind": "material_step", "trace_id": null}}))
}
fn open(port: u16, path: &str, headers: &str) -> TcpStream {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n{headers}\r\n").unwrap();
    s
}
/// Reads until `done(text)` or 5 s.
fn read_until(s: &mut TcpStream, done: impl Fn(&str) -> bool) -> String {
    let (mut text, t0) = (String::new(), Instant::now());
    let mut buf = [0u8; 4096];
    while !done(&text) && t0.elapsed() < Duration::from_secs(5) {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => text.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(_) => {}
        }
    }
    text
}
fn ids(text: &str) -> Vec<i64> {
    text.lines().filter_map(|l| l.strip_prefix("id: ")?.parse().ok()).collect()
}

#[test]
fn last_event_id_resumes_without_loss_or_duplication_then_follows_live_appends() {
    let (store, port) = boot(5000);
    store.emit("run-active", NewEvent::new("run_started", "run", "run-active", json!({"title": "t", "state": "running", "origin": "manual"}))).unwrap();
    for i in 0..2 {
        store.emit("run-active", ev(i)).unwrap();
    }
    let mut s = open(port, &format!("{D}/runs/run-active/events/stream"), "Last-Event-ID: 1\r\n");
    let text = read_until(&mut s, |t| t.contains("id: 3"));
    assert!(text.starts_with("HTTP/1.1 200") && text.contains("text/event-stream"), "{text}");
    assert_eq!(ids(&text), vec![2, 3]);
    for l in text.lines().filter_map(|l| l.strip_prefix("data: ")) {
        let v: Value = serde_json::from_str(l).unwrap();
        assert_eq!(v["run_id"], "run-active");
        assert!(ids(&text).contains(&v["sequence"].as_i64().unwrap()));
    }
    store.emit("run-active", ev(2)).unwrap(); // a running engine appends while the client is connected
    let more = read_until(&mut s, |t| t.contains("id: 4"));
    assert_eq!(ids(&more), vec![4]);
}

#[test]
fn spec25_stream_path_and_after_sequence_query_resume_too() {
    let (store, port) = boot(5000);
    store.emit("r", NewEvent::new("run_started", "run", "r", json!({"title": "t", "state": "running", "origin": "manual"}))).unwrap();
    store.emit("r", ev(0)).unwrap();
    let mut s = open(port, &format!("{D}/runs/r/stream?after_sequence=1"), "");
    assert_eq!(ids(&read_until(&mut s, |t| t.contains("id: 2"))), vec![2]);
}

#[test]
fn heartbeat_comment_frames_arrive_at_the_configured_cadence() {
    let (store, port) = boot(100);
    store.emit("r", NewEvent::new("run_started", "run", "r", json!({"title": "t", "state": "running", "origin": "manual"}))).unwrap();
    let mut s = open(port, &format!("{D}/runs/r/events/stream"), "");
    let text = read_until(&mut s, |t| t.matches(": hb").count() >= 2);
    assert!(text.matches(": hb").count() >= 2, "{text}");
}

#[test]
fn purged_cursor_is_a_410_json_problem_over_the_wire() {
    let (store, port) = boot(5000);
    store.emit("r", NewEvent::new("run_started", "run", "r", json!({"title": "t", "state": "running", "origin": "manual"}))).unwrap();
    store.emit("r", ev(0)).unwrap();
    store.emit("r", ev(1)).unwrap();
    store.purge_through("r", 2).unwrap();
    let mut s = open(port, &format!("{D}/runs/r/events/stream"), "Last-Event-ID: 1\r\n");
    let text = read_until(&mut s, |t| t.contains("cursor_expired"));
    assert!(text.starts_with("HTTP/1.1 410"), "{text}");
    let b: Value = serde_json::from_str(text.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!((b["code"].as_str(), b["recovery_after_sequence"].as_i64(), b["snapshot_url"].as_str()), (Some("cursor_expired"), Some(3), Some("/internal/v1/debug/runs/r/graph")));
}

#[test]
fn json_routes_work_over_the_wire() {
    let (store, port) = boot(5000);
    store.emit("r", NewEvent::new("run_started", "run", "r", json!({"title": "t", "state": "running", "origin": "manual"}))).unwrap();
    let mut s = open(port, &format!("{D}/runs"), "Connection: close\r\n");
    let text = read_until(&mut s, |t| t.contains("\"items\""));
    assert!(text.starts_with("HTTP/1.1 200") && text.to_lowercase().contains("application/json"), "{text}");
}

#[test]
fn only_loopback_addresses_can_be_bound() {
    assert!(bind_loopback("0.0.0.0:0").is_err());
    assert!(bind_loopback("192.168.1.10:0").is_err());
    assert!(bind_loopback("not an address").is_err());
    assert!(bind_loopback("127.0.0.1:0").is_ok());
}

#[test]
fn a_purge_past_a_connected_cursor_ends_the_feed_instead_of_skipping_events() {
    let (store, port) = boot(5000);
    store.emit("r", NewEvent::new("run_started", "run", "r", json!({"title": "t", "state": "running", "origin": "manual"}))).unwrap();
    let mut s = open(port, &format!("{D}/runs/r/events/stream"), "");
    read_until(&mut s, |t| t.contains("id: 1"));
    for i in 0..3 {
        store.emit("r", ev(i)).unwrap();
    }
    store.purge_through("r", 3).unwrap(); // events 2 and 3 are gone before the feed could send them
    store.emit("r", ev(3)).unwrap(); // 5? no: head 5 -> seq 5
    let text = read_until(&mut s, |t| t.contains("id: 5"));
    assert!(!ids(&text).contains(&5) || ids(&text).contains(&2), "silent gap: {:?}", ids(&text));
}
