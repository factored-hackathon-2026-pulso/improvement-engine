//! Process-level: spawn the real `pulso` binary in memory mode, probe it, terminate it, check the exit code and that
//! nothing is left listening. Termination here is stdin EOF (the Windows-portable path, `--exit-on-stdin-eof`); on unix
//! SIGTERM is exercised too.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pulso"));
    for (k, _) in std::env::vars() {
        if k.starts_with("PULSO_") {
            c.env_remove(k);
        }
    }
    c
}

struct Proc {
    child: Child,
    addr: SocketAddr,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(extra_env: &[(&str, &str)], args: &[&str]) -> Proc {
    let mut c = bin();
    c.arg("run").args(args).env("PULSO_STORAGE", "memory").env("PULSO_DATA_MODE", "dataset").env("PULSO_LISTEN_ADDR", "127.0.0.1:0").env("PULSO_POLL_INTERVAL_MS", "50");
    for (k, v) in extra_env {
        c.env(k, v);
    }
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).unwrap() > 0, "process ended before listening");
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap_or_else(|_| panic!("stdout must be JSON lines only: {line}"));
        if v["event"] == "listening" {
            let addr = v["addr"].as_str().unwrap().parse().unwrap();
            return Proc { child, addr, stdout };
        }
        assert!(Instant::now() < deadline, "never listened");
    }
}

fn status(addr: SocketAddr, path: &str) -> u16 {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    raw.split(' ').nth(1).unwrap().parse().unwrap()
}

fn wait_ready(addr: SocketAddr) {
    let t = Instant::now();
    while status(addr, "/readyz") != 200 {
        assert!(t.elapsed() < Duration::from_secs(10), "never ready");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_exit(p: &mut Proc, secs: u64) -> i32 {
    let t = Instant::now();
    loop {
        if let Some(st) = p.child.try_wait().unwrap() {
            return st.code().unwrap_or(-1);
        }
        assert!(t.elapsed() < Duration::from_secs(secs), "process did not exit in {secs}s");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn runs_ready_then_exits_zero_on_stdin_eof_and_leaves_nothing_listening() {
    let mut p = spawn(&[], &["--exit-on-stdin-eof"]);
    assert_eq!(status(p.addr, "/healthz"), 200);
    wait_ready(p.addr);
    drop(p.child.stdin.take()); // EOF
    assert_eq!(wait_exit(&mut p, 10), 0);
    assert!(TcpStream::connect_timeout(&p.addr, Duration::from_millis(300)).is_err(), "no leftover listener");
    let mut rest = String::new();
    p.stdout.read_to_string(&mut rest).unwrap();
    assert!(rest.contains("shutdown_end"), "{rest}");
    for line in rest.lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap_or_else(|_| panic!("non-JSON log line: {line}"));
    }
}

#[test]
fn without_the_flag_a_closed_stdin_does_not_stop_a_container_process() {
    let mut p = spawn(&[], &[]);
    drop(p.child.stdin.take());
    std::thread::sleep(Duration::from_millis(600));
    assert!(p.child.try_wait().unwrap().is_none(), "ECS gives stdin /dev/null; that must not be a stop signal");
    wait_ready(p.addr);
}

#[test]
fn the_healthcheck_subcommand_follows_readiness() {
    let mut p = spawn(&[], &["--exit-on-stdin-eof"]);
    wait_ready(p.addr);
    let port = p.addr.port().to_string();
    let hc = || bin().args(["healthcheck", "--port", &port]).status().unwrap().code().unwrap();
    assert_eq!(hc(), 0);
    drop(p.child.stdin.take());
    wait_exit(&mut p, 10);
    assert_eq!(hc(), 1);
}

#[test]
fn invalid_config_is_refused_with_a_named_reason_and_no_secret() {
    let cases: [(&[(&str, &str)], &str); 3] = [
        (&[("PULSO_LISTEN_ADDR", "0.0.0.0:8080")], "PULSO_ALLOW_NON_LOOPBACK"),
        (&[("PULSO_SOURCE_ADAPTER", "product-postgres")], "config_conflict"),
        (&[("PULSO_STORAGE", ""), ("PULSO_DATABASE_URL", "postgres://svc:topsecret@db.invalid/pulso"), ("PULSO_DATA_MODE", "bogus")], "PULSO_DATA_MODE"),
    ];
    for (env, want) in cases {
        let mut c = bin();
        c.arg("run").env("PULSO_STORAGE", "memory").env("PULSO_DATA_MODE", "dataset");
        for (k, v) in env {
            c.env(k, v);
        }
        let out = c.stdin(Stdio::null()).output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert_eq!(out.status.code(), Some(2), "{want}: {text}");
        assert!(text.contains(want), "{want}: {text}");
        assert!(!text.contains("topsecret") && !text.contains("db.invalid"), "{text}");
    }
}

#[test]
fn an_unreachable_database_keeps_the_process_up_but_not_ready() {
    let mut c = bin();
    c.arg("run")
        .env("PULSO_DATABASE_URL", "postgres://svc:topsecret@127.0.0.1:1/pulso?connect_timeout=1")
        .env("PULSO_DATA_MODE", "dataset")
        .env("PULSO_LISTEN_ADDR", "127.0.0.1:0");
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut addr = None;
    let mut all = String::new();
    for _ in 0..50 {
        let mut line = String::new();
        if out.read_line(&mut line).unwrap() == 0 {
            break;
        }
        all.push_str(&line);
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        if v["event"] == "listening" {
            addr = Some(v["addr"].as_str().unwrap().parse::<SocketAddr>().unwrap());
            break;
        }
    }
    let addr = addr.unwrap_or_else(|| panic!("no listening event: {all}"));
    assert_eq!(status(addr, "/healthz"), 200);
    let t = Instant::now();
    while status(addr, "/readyz") == 200 {
        assert!(t.elapsed() < Duration::from_secs(2), "must not be ready");
    }
    assert_eq!(status(addr, "/readyz"), 503);
    let _ = child.kill();
    let mut rest = String::new();
    let _ = out.read_to_string(&mut rest);
    let _ = child.wait();
    assert!(!(all + &rest).contains("topsecret"));
}

#[cfg(unix)]
#[test]
fn sigterm_exits_zero() {
    let mut p = spawn(&[], &[]);
    wait_ready(p.addr);
    let pid = p.child.id().to_string();
    assert!(Command::new("kill").args(["-TERM", &pid]).status().unwrap().success());
    assert_eq!(wait_exit(&mut p, 10), 0);
}
