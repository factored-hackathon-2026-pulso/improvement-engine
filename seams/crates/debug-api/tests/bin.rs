//! The binary: boots on loopback, prints its URL, serves ingested reports and the contract seed, refuses non-loopback.
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/engine-run-demo0.json");

struct Proc(Child);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(args: &[&str], envs: &[(&str, &str)]) -> (Proc, u16) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_debug-api"));
    c.args(args).envs(envs.iter().copied()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = c.spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let port = line.trim().rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or_else(|| panic!("no listening line: {line:?}"));
    (Proc(child), port)
}
fn get(port: u16, path: &str, extra: &str) -> (String, Value) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{extra}\r\n").unwrap();
    let mut t = String::new();
    s.read_to_string(&mut t).unwrap();
    let (head, body) = t.split_once("\r\n\r\n").unwrap_or((&t, ""));
    (head.lines().next().unwrap_or_default().to_string(), serde_json::from_str(body).unwrap_or(Value::Null))
}

#[test]
fn serves_an_ingested_report_and_the_contract_seed_from_a_file_store() {
    let dir = std::env::temp_dir().join(format!("debug-api-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (_p, port) = spawn(&["--addr", "127.0.0.1:0", "--store-dir", dir.to_str().unwrap(), "--ingest", FIXTURE, "--seed-contract"], &[]);
    let (status, runs) = get(port, "/internal/v1/debug/runs", "");
    assert!(status.contains("200"), "{status}");
    let ids: Vec<&str> = runs["items"].as_array().unwrap().iter().map(|r| r["run_id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["run-demo-0-aa2f9472", "run-active"]);
    let (_, prof) = get(port, "/internal/v1/debug/profile", "");
    assert_eq!(prof["mode"], "stand_in");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restart_on_the_same_store_dir_does_not_ingest_twice() {
    let dir = std::env::temp_dir().join(format!("debug-api-bin2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let args = ["--addr", "127.0.0.1:0", "--store-dir", dir.to_str().unwrap(), "--ingest", FIXTURE];
    drop(spawn(&args, &[]));
    let (_p, port) = spawn(&args, &[]);
    let (_, runs) = get(port, "/internal/v1/debug/runs", "");
    assert_eq!(runs["items"].as_array().unwrap().len(), 1);
    assert_eq!(runs["items"][0]["projection_revision"], 15, "start + doubles + 11 steps + gates + completed, counted once");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn static_token_is_enforced_when_set() {
    let (_p, port) = spawn(&["--addr", "127.0.0.1:0"], &[("DEBUG_API_TOKEN", "dev-token")]);
    assert!(get(port, "/internal/v1/debug/runs", "").0.contains("401"));
    assert!(get(port, "/internal/v1/debug/runs", "Authorization: Bearer dev-token\r\n").0.contains("200"));
}

#[test]
fn refuses_a_non_loopback_address() {
    let st = Command::new(env!("CARGO_BIN_EXE_debug-api")).args(["--addr", "0.0.0.0:0"]).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
    assert_eq!(st.code(), Some(2));
}
