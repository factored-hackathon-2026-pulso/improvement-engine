//! SSE run-event feed (DebugApi `openStream` contract, `debug-console/src/api/debug/stream.ts`): resume with
//! `after_sequence` / `Last-Event-ID`, `410 cursor_expired` with a recovery cursor for an unknown or purged cursor,
//! 404 for a run the caller cannot see, heartbeats, live append. Rust-only (the Python double has no debug feed).
mod common;
use common::*;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn start(rig: &Rig) -> String {
    let s = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = s.server_addr().to_ip().unwrap().to_string();
    let app = rig.app.clone();
    std::thread::spawn(move || control_api::server::serve(s, app));
    addr
}

fn ev(seq: i64, run: &str) -> Value {
    json!({"sequence": seq, "run_ref": run, "job_ref": null, "stage": "scout", "event_code": "stage.completed", "status": "ok", "reason_code": null,
           "artifact_ref": null, "trace_id": null, "details_ref": null, "occurred_at": "2026-03-01T10:00:00Z"})
}

fn seed(rig: &Rig, tenant: &str, run: &str, floor: i64, seqs: &[i64]) {
    rig.admin(json!({"run_events": [{"tenant": tenant, "run": run, "floor": floor, "events": seqs.iter().map(|s| ev(*s, run)).collect::<Vec<_>>()}]}));
}

struct Stream(TcpStream, String);

fn open(addr: &str, rig: &Rig, tenant: &str, scope: &str, run: &str, query: &str, last_id: Option<&str>) -> Stream {
    let tok = rig.token("cb", "control-api", scope, tenant, json!({"purpose": scope}));
    let mut c = TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
    let mut req = format!("GET /internal/v1/debug/runs/{run}/stream{query} HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {tok}\r\nAccept: text/event-stream\r\n");
    if let Some(id) = last_id {
        req.push_str(&format!("Last-Event-ID: {id}\r\n"));
    }
    c.write_all(format!("{req}\r\n").as_bytes()).unwrap();
    Stream(c, String::new())
}

impl Stream {
    /// Reads until `done(buffer)` or the deadline; returns the accumulated text (head included).
    fn until(&mut self, secs: u64, done: impl Fn(&str) -> bool) -> &str {
        let end = Instant::now() + Duration::from_secs(secs);
        let mut buf = [0u8; 4096];
        while Instant::now() < end && !done(&self.1) {
            match self.0.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.1.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => {}
            }
        }
        &self.1
    }

    fn ids(&self) -> Vec<i64> {
        self.1.lines().filter_map(|l| l.strip_prefix("id: ")?.trim().parse().ok()).collect()
    }
}

#[test]
fn stream_delivers_only_events_after_the_cursor_in_order_and_heartbeats() {
    let rig = Rig::new();
    seed(&rig, "t1", "run-1", 0, &[1, 2, 3]);
    let addr = start(&rig);
    let mut s = open(&addr, &rig, "t1", "debug_read", "run-1", "?after_sequence=1", None);
    let text = s.until(5, |t| t.contains("id: 3") && t.contains(": hb")).to_string();
    assert!(text.starts_with("HTTP/1.1 200") && text.contains("text/event-stream"), "{text}");
    assert_eq!(s.ids(), vec![2, 3], "{text}");
    let data = text.lines().find_map(|l| l.strip_prefix("data: ")).unwrap();
    let first: Value = serde_json::from_str(data).unwrap();
    assert_eq!((first["sequence"].as_i64(), first["run_ref"].as_str()), (Some(2), Some("run-1")));
    assert!(text.contains(": hb"), "a heartbeat comment proves silence is not completion: {text}");
}

#[test]
fn last_event_id_resumes_and_live_appends_arrive() {
    let rig = Rig::new();
    seed(&rig, "t1", "run-1", 0, &[1, 2, 3]);
    let addr = start(&rig);
    let mut s = open(&addr, &rig, "t1", "debug_read", "run-1", "", Some("2"));
    s.until(5, |t| t.contains("id: 3"));
    assert_eq!(s.ids(), vec![3]);
    seed(&rig, "t1", "run-1", 0, &[4]); // appended while the client is connected
    s.until(5, |t| t.contains("id: 4"));
    assert_eq!(s.ids(), vec![3, 4]);
}

#[test]
fn unknown_or_purged_cursor_is_410_with_a_recovery_cursor_inside_the_run() {
    let rig = Rig::new();
    seed(&rig, "t1", "run-1", 5, &[6, 7, 8]); // sequences 1..=5 were purged
    let addr = start(&rig);
    for (query, recovery) in [("?after_sequence=2", 5), ("?after_sequence=99", 8)] {
        let mut s = open(&addr, &rig, "t1", "debug_read", "run-1", query, None);
        let text = s.until(5, |t| t.contains("\r\n\r\n") && t.ends_with('}')).to_string();
        assert!(text.starts_with("HTTP/1.1 410"), "{query}: {text}");
        let body: Value = serde_json::from_str(&text[text.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(body["code"], "cursor_expired");
        assert_eq!(body["recovery"]["after_sequence"], recovery, "{query}: {body}");
        assert!(body["recovery"]["snapshot_ref"].as_str().unwrap().contains("/runs/run-1/"), "a snapshot ref never leaves its run");
    }
    // cursor at the floor is fine: nothing purged after it
    let mut s = open(&addr, &rig, "t1", "debug_read", "run-1", "?after_sequence=5", None);
    s.until(5, |t| t.contains("id: 8"));
    assert_eq!(s.ids(), vec![6, 7, 8]);
}

#[test]
fn another_tenants_run_is_404_and_auth_is_enforced() {
    let rig = Rig::new();
    seed(&rig, "t1", "run-1", 0, &[1]);
    let addr = start(&rig);
    let status = |s: &mut Stream| s.until(5, |t| t.contains("\r\n")).lines().next().unwrap_or_default().to_string();
    assert!(status(&mut open(&addr, &rig, "t2", "debug_read", "run-1", "", None)).contains("404"));
    assert!(status(&mut open(&addr, &rig, "t1", "debug_read", "missing", "", None)).contains("404"));
    assert!(status(&mut open(&addr, &rig, "t1", "binding", "run-1", "", None)).contains("403"));
    let mut anon = TcpStream::connect(&addr).unwrap();
    anon.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    anon.write_all(b"GET /internal/v1/debug/runs/run-1/stream HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    let mut out = [0u8; 64];
    let n = anon.read(&mut out).unwrap();
    assert!(String::from_utf8_lossy(&out[..n]).starts_with("HTTP/1.1 401"));
    assert!(status(&mut open(&addr, &rig, "t1", "debug_read", "run-1", "?after_sequence=abc", None)).contains("400"));
}
