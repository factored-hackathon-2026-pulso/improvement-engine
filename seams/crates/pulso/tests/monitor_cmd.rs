use pulso::monitor_cmd::parse;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PULSO: &str = env!("CARGO_BIN_EXE_pulso");
const RUNNER: &str = env!("CARGO_BIN_EXE_pulso-synth-runner");

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_owned()).collect()
}
fn no_env(_: &str) -> Option<String> {
    None
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pulso-monitor-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Synthetic platform-shaped SQLite (invented values): 7 events, a credential table and a payload sentinel.
fn fixture(dir: &Path) -> PathBuf {
    let p = dir.join("platform.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute_batch(
        "CREATE TABLE event_log(sequence INTEGER PRIMARY KEY, event_id TEXT, event_type TEXT, entity TEXT, entity_id TEXT, case_id TEXT, actor_role TEXT, actor_id TEXT, event_time TEXT, ingested_at TEXT, payload TEXT, tenant_id TEXT);
         CREATE TABLE login_accounts(id TEXT, password_hash TEXT);
         INSERT INTO login_accounts VALUES('l1','SECRET-PW-HASH');",
    )
    .unwrap();
    for n in 1..=7 {
        let t = if n % 3 == 0 { "case.closed" } else { "case.opened" };
        c.execute("INSERT INTO event_log VALUES(?,?,?,?,?,?,?,?,?,?,?,?)", rusqlite::params![n, format!("ev-{n}"), t, "case", format!("c{n}"), format!("c{n}"), "customer", "a", "2026-10-01T10:00:00Z", "x", "SECRET-PAYLOAD", "t"]).unwrap();
    }
    p
}

fn monitor(args: &[&str]) -> Command {
    let mut c = Command::new(PULSO);
    c.arg("monitor").args(args).env("STEPS_RUNNER_EXE", RUNNER).env_remove("PULSO_DATA_MODE").env_remove("PULSO_SOURCE_ADAPTER").env_remove("PULSO_SOURCE_ID");
    c
}

fn base<'a>(work: &'a str, db: &'a str) -> Vec<&'a str> {
    vec!["--data-mode", "platform", "--adapter", "product-sqlite", "--source-id", "platform:tenant-a", "--sqlite", db, "--work-dir", work]
}

#[test]
fn parse_reads_flags_and_env_and_flags_win() {
    let env = |k: &str| match k {
        "PULSO_DATA_MODE" => Some("platform".to_owned()),
        "PULSO_SOURCE_ADAPTER" => Some("product-sqlite".to_owned()),
        "PULSO_SOURCE_ID" => Some("platform:from-env".to_owned()),
        "PULSO_SOURCE_SQLITE" => Some("env.db".to_owned()),
        "PULSO_POLL_INTERVAL_SECS" => Some("9".to_owned()),
        "STEPS_RUNNER_EXE" => Some("runner".to_owned()),
        _ => None,
    };
    let a = parse(&s(&["--work-dir", "w", "--source-id", "platform:flag", "--once", "--batch-cap", "7"]), &env).unwrap();
    assert!(a.once);
    assert_eq!(a.config.source_id.as_str(), "platform:flag");
    assert_eq!((a.config.batch_cap, a.config.poll_interval), (7, Duration::from_secs(9)));
    assert_eq!(a.sqlite.as_deref(), Some(Path::new("env.db")));
    assert!(!parse(&s(&["--work-dir", "w"]), &env).unwrap().once);
}

#[test]
fn parse_refuses_unknown_missing_or_inconsistent_input() {
    let ok = ["--data-mode", "platform", "--adapter", "product-sqlite", "--source-id", "platform:a", "--sqlite", "x.db", "--work-dir", "w", "--runner", "r"];
    assert!(parse(&s(&ok), &no_env).is_ok());
    for extra in [vec!["--frobnicate"], vec!["--batch-cap", "0"], vec!["--poll-interval-secs", "0"], vec!["--data-mode", "dataset"], vec!["--adapter", "product-mysql"]] {
        let mut v = ok.to_vec();
        v.extend(extra.iter());
        assert!(parse(&s(&v), &no_env).is_err(), "{extra:?}");
    }
    let no_sqlite: Vec<&str> = ok.iter().copied().filter(|x| *x != "--sqlite" && *x != "x.db").collect();
    assert!(parse(&s(&no_sqlite), &no_env).is_err(), "product-sqlite needs --sqlite");
    let no_work: Vec<&str> = ok.iter().copied().filter(|x| *x != "--work-dir" && *x != "w").collect();
    assert!(parse(&s(&no_work), &no_env).is_err(), "work dir is explicit");
}

#[test]
fn usage_errors_exit_2_without_touching_anything() {
    let out = monitor(&[]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("pulso monitor"));
    assert_eq!(monitor(&["--frobnicate"]).output().unwrap().status.code(), Some(2));
}

#[test]
fn once_runs_one_tick_then_idles_without_leaking_denied_data() {
    let dir = temp("once");
    let db = fixture(&dir);
    let work = dir.join("work");
    let (w, d) = (work.to_str().unwrap(), db.to_str().unwrap());
    let mut args = base(w, d);
    args.push("--once");
    let out = monitor(&args).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(0), "{stdout} {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("run=mon-") && stdout.contains("events=7") && stdout.contains("watermark=seq:7"), "{stdout}");
    assert!(stdout.contains("data_mode=platform") && stdout.contains("source=platform:tenant-a"), "{stdout}");
    let again = String::from_utf8_lossy(&monitor(&args).output().unwrap().stdout).into_owned();
    assert!(again.contains("idle"), "{again}");
    assert_eq!(std::fs::read_dir(work.join("runs")).unwrap().count(), 1);
    for f in std::fs::read_dir(work.join("runs")).unwrap() {
        let t = std::fs::read_to_string(f.unwrap().path()).unwrap();
        assert!(t.contains("\"data_origin\":\"platform_live\"") && !t.contains("SECRET"));
    }
}

#[test]
fn dataset_mode_with_a_product_adapter_exits_2() {
    let dir = temp("refuse");
    let db = fixture(&dir);
    let work = dir.join("work");
    let out = monitor(&["--data-mode", "dataset", "--adapter", "product-sqlite", "--source-id", "dataset:e0:x", "--sqlite", db.to_str().unwrap(), "--work-dir", work.to_str().unwrap(), "--once"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!work.join("runs").exists());
}

#[test]
fn loop_ticks_on_an_interval_and_exits_0_when_stdin_closes() {
    let dir = temp("loop");
    let db = fixture(&dir);
    let work = dir.join("work");
    let mut args = base(work.to_str().unwrap(), db.to_str().unwrap());
    args.extend(["--poll-interval-secs", "1"]);
    let mut child = monitor(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(2500));
    assert!(child.try_wait().unwrap().is_none(), "the loop keeps running");
    drop(child.stdin.take()); // EOF
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        assert!(Instant::now() < deadline, "did not exit on stdin EOF");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(0));
    let mut out = String::new();
    child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    assert!(out.contains("events=7"), "{out}");
    assert!(out.matches("idle").count() >= 1, "a second tick ran and found nothing new: {out}");
}
