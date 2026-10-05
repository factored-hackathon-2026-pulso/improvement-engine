//! `pulso loop` as a process: the exit codes systemd keys on, the run lock, the structured log, and no secret on any stream.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXE: &str = env!("CARGO_BIN_EXE_pulso");
const SEED: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";
const GATEWAY_KEY: &str = "gw-canary-9f3b-not-a-real-key";

fn work(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pulso-loopproc-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("inputs")).unwrap();
    std::fs::create_dir_all(d.join("work")).unwrap();
    d
}

fn base(root: &Path) -> Command {
    let mut c = Command::new(EXE);
    c.arg("loop")
        .env_clear()
        .env("PULSO_WORK_DIR", root.join("work"))
        .env("PULSO_LOOP_INPUTS_DIR", root.join("inputs"))
        .env("PULSO_CELLS_SOURCE", "synthetic")
        .env("PULSO_CORE_ADDR", "127.0.0.1:1")
        .env("PULSO_SERVICE_SEED_HEX", SEED)
        .env("PULSO_SERVICE_KID", "pulso-engine-1")
        .env("PULSO_EVAL_BEFORE_ANNOUNCE", "off")
        .env("PULSO_LLM_GATEWAY", "enabled")
        .env("PULSO_LLM_GATEWAY_ADDR", "127.0.0.1:1")
        .env("PULSO_LLM_GATEWAY_KEY", GATEWAY_KEY);
    c
}

fn streams(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn assert_no_secret(o: &Output) {
    let all = streams(o);
    assert!(!all.contains(SEED) && !all.contains(GATEWAY_KEY), "a secret reached a stream: {all}");
}

#[test]
fn a_refused_configuration_exits_2_and_names_the_variable() {
    let r = work("refused");
    let o = base(&r).env_remove("PULSO_SERVICE_SEED_HEX").output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(streams(&o).contains("PULSO_SERVICE_SEED_HEX"), "{}", streams(&o));
    assert_no_secret(&o);
    let o = base(&r).env_remove("PULSO_WORK_DIR").output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let o = Command::new(EXE).args(["loop", "--bogus"]).env_clear().output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn the_demo_profile_is_refused_unless_the_data_is_synthetic() {
    let r = work("demo");
    let o = base(&r).env("PULSO_PROFILE", "demo").env("PULSO_CELLS_SOURCE", "bank").output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(streams(&o).contains("PULSO_PROFILE=demo"), "{}", streams(&o));
    let o = base(&r).env("PULSO_PROFILE", "demo").arg("--check").output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", streams(&o));
    assert!(streams(&o).contains("\"support_profile\":\"demo\""));
}

#[test]
fn check_validates_without_running_and_logs_structured_json_without_secrets() {
    let r = work("check");
    let o = base(&r).arg("--check").output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", streams(&o));
    let line = String::from_utf8_lossy(&o.stdout).lines().find(|l| l.contains("loop_check")).expect("a loop_check line").to_string();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["event"], "loop_check");
    assert_eq!(v["cells_present"], false);
    assert_eq!(v["support_profile"], "standard");
    assert_no_secret(&o);
    assert!(!r.join("work").join("loop.lock").exists(), "--check takes no lock");
}

#[test]
fn missing_cells_exit_1_and_release_the_lock() {
    let r = work("nocells");
    let o = base(&r).output().unwrap();
    assert_eq!(o.status.code(), Some(1), "{}", streams(&o));
    assert!(streams(&o).contains("cells_missing"));
    assert!(!r.join("work").join("loop.lock").exists(), "the lock is released when the run ends");
    assert_no_secret(&o);
}

#[test]
fn a_second_run_while_one_holds_the_lock_exits_75_and_touches_nothing() {
    let r = work("locked");
    std::fs::write(r.join("work").join("loop.lock"), "{\"pid\":1}").unwrap();
    let o = base(&r).output().unwrap();
    assert_eq!(o.status.code(), Some(75), "{}", streams(&o));
    assert!(streams(&o).contains("loop_locked"));
    assert!(r.join("work").join("loop.lock").exists(), "the holder's lock is not removed by the refused run");
    assert!(!r.join("work").join("loop-latest.json").exists());
    // an operator can break a lock left by a killed run
    let o = base(&r).arg("--break-lock").output().unwrap();
    assert_eq!(o.status.code(), Some(1), "after the lock is broken the run proceeds (and finds no cells): {}", streams(&o));
}

#[test]
fn a_planted_cells_run_with_an_unreachable_gateway_finishes_with_exit_3_and_a_closed_record() {
    // Real cells, the real sensor; the gateway is unreachable on purpose: the finding ends `blocked(model_unavailable)`, an infrastructure
    // failure, so the job exits 3 (systemd: failed, the timer retries and the finding is reasoned again); the result is written.
    let r = work("unreach");
    std::fs::write(r.join("inputs").join("cells.ndjson"), reasoning::testkit::synthetic_cells_ndjson()).unwrap();
    let o = base(&r).output().unwrap();
    assert_eq!(o.status.code(), Some(3), "{}", streams(&o));
    let all = streams(&o);
    assert!(all.contains("loop_start") && all.contains("loop_finding") && all.contains("loop_done") && all.contains("\"infra_failures\":1"), "{all}");
    assert!(!r.join("work").join("loop.lock").exists());
    let latest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(r.join("work").join("loop-latest.json")).unwrap()).unwrap();
    assert_eq!(latest["summary"]["blocked"], 1);
    assert_eq!(latest["support_profile"], "standard");
    assert_no_secret(&o);
}
