//! The `pulso` binary: `serve` (embedded debug-api, loopback, console static files, config.json pointing at itself) and
//! `demo` (streams the ten-step run into that server, prints doubles first). Real processes, real sockets.
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_pulso");

struct Proc(Child);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve(extra: &[&str]) -> (Proc, String) {
    let mut c = Command::new(BIN);
    c.arg("serve").args(["--addr", "127.0.0.1:0"]).args(extra).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = c.spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let addr = line.trim().strip_prefix("pulso listening on http://").unwrap_or_else(|| panic!("no listening line: {line:?}")).to_string();
    (Proc(child), addr)
}

fn get(addr: &str, path: &str) -> (u16, String) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
    let mut t = String::new();
    s.read_to_string(&mut t).unwrap();
    let (head, body) = t.split_once("\r\n\r\n").unwrap_or((&t, ""));
    (head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0), body.to_string())
}

fn dist() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pulso-cli-dist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("index.html"), "<html>console-built</html>").unwrap();
    std::fs::write(d.join("config.json"), r#"{"provider":"fixture"}"#).unwrap();
    d
}

#[test]
fn serve_points_the_console_at_itself_and_serves_the_built_files() {
    let d = dist();
    let (_p, addr) = serve(&["--console-dir", d.to_str().unwrap(), "--admin-token", "t"]);
    assert_eq!(get(&addr, "/").1, "<html>console-built</html>");
    let cfg: Value = serde_json::from_str(&get(&addr, "/config.json").1).unwrap();
    assert_eq!(cfg["dataProvider"], "http");
    assert_eq!(cfg["apiBase"], "", "same origin: no proxy needed");
    assert_eq!(get(&addr, "/healthz").0, 200);
}

#[test]
fn serve_without_a_console_dir_serves_the_api_only_and_says_so() {
    let (_p, addr) = serve(&["--console-dir", "Z:/definitely/not/here", "--admin-token", "t"]);
    assert_eq!(get(&addr, "/").0, 404);
    assert_eq!(get(&addr, "/healthz").0, 200);
}

#[test]
fn serve_refuses_a_non_loopback_address() {
    let o = Command::new(BIN).args(["serve", "--addr", "0.0.0.0:4020"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("loopback"));
}

#[test]
fn demo_streams_into_serve_and_prints_doubles_before_steps() {
    let (_p, addr) = serve(&["--admin-token", "t"]);
    let out = Command::new(BIN)
        .args(["demo", "--api", &addr, "--admin-token", "t", "--pace-ms", "0", "--run-id", "run-demo0-cli"])
        .env("PULSO_WORK_DIR", std::env::temp_dir().join(format!("pulso-cli-work-{}", std::process::id())))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code(), Some(0), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    let (d, s) = (text.find("NOT REAL (doubles[]").unwrap(), text.find("STEPS (id, status").unwrap());
    assert!(d < s, "doubles first");
    assert!(text.contains("DEMO-0") && text.contains("recompute: real-narrow"), "{text}");
    assert!(text.contains("run-demo0-cli"), "names the run so the console can open it: {text}");
    let runs: Value = serde_json::from_str(&get(&addr, "/internal/v1/debug/runs").1).unwrap();
    let item = runs["items"].as_array().unwrap().iter().find(|r| r["run_id"] == "run-demo0-cli").expect("run listed");
    assert_eq!(item["state"], "completed");
    let g: Value = serde_json::from_str(&get(&addr, "/internal/v1/debug/runs/run-demo0-cli/graph").1).unwrap();
    assert_eq!(g["nodes"].as_array().unwrap().len(), 12);
}

#[test]
fn demo_real_core_refuses_honestly_without_running_anything() {
    let o = Command::new(BIN).args(["demo", "--real-core"]).env_remove("PULSO_CORE_URL").output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("real Core") && err.contains("refus"), "{err}");
    assert!(o.stdout.is_empty(), "nothing is printed as if it ran");
}

#[test]
fn demo_against_an_unreachable_api_fails_with_a_clear_message() {
    let o = Command::new(BIN).args(["demo", "--api", "127.0.0.1:1", "--admin-token", "t"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("pulso serve"), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn serve_exits_when_its_stdin_closes_so_a_killed_parent_leaks_nothing() {
    let mut c = Command::new(BIN);
    c.args(["serve", "--addr", "127.0.0.1:0", "--admin-token", "t", "--console-dir", "Z:/none", "--exit-on-stdin-eof"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = c.spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    assert!(line.starts_with("pulso listening on"), "{line:?}");
    assert!(child.try_wait().unwrap().is_none(), "must keep serving while stdin is open");
    drop(child.stdin.take()); // what the OS does when the parent is hard-killed
    for _ in 0..100 {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    panic!("serve outlived the closed stdin");
}
