//! Q1: resume after kill -9 of the thread10 process over the engine FileStore. The Pg store conformance needs a live
//! database and is NOT run here (offline slice): the file store is the only store exercised.
use serde_json::Value;
use std::process::{Command, Stdio};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("t10r-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn cmd(work: &std::path::Path, now: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_thread10"));
    c.arg(work).arg("--override").arg("--now").arg(now).env("STEPS_RUNNER_EXE", env!("CARGO_BIN_EXE_synth_runner"));
    c
}

fn report(out: &std::process::Output) -> Value {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("report json on stdout")
}

fn kill_at(idx: usize, name: &str) -> (Value, Value) {
    let clean = report(&cmd(&tmp(&format!("{name}-clean")), "1000").output().unwrap());
    let work = tmp(&format!("{name}-kill"));
    let marker = work.with_extension("marker");
    let _ = std::fs::remove_file(&marker);
    let mut child = cmd(&work, "1000").arg("--marker").arg(&marker).arg(idx.to_string()).stdout(Stdio::null()).spawn().unwrap();
    for _ in 0..1500 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists(), "handler {idx} never committed");
    child.kill().unwrap(); // hard kill, no cleanup
    child.wait().unwrap();
    let resumed = report(&cmd(&work, "1060").output().unwrap()); // later clock: the dead worker's lease expired
    (clean, resumed)
}

fn same_outcome(clean: &Value, resumed: &Value) {
    assert_eq!(clean["events"], resumed["events"], "identical committed event sequence");
    let key = |r: &Value| r["steps"].as_array().unwrap().iter().map(|s| format!("{}:{}:{}", s["n"], s["id"], s["status"])).collect::<Vec<_>>();
    assert_eq!(key(clean), key(resumed));
    assert_eq!(clean["successor"]["unique_key"], resumed["successor"]["unique_key"], "same release id, one successor");
    assert_eq!(resumed["successor"]["runs"], 1);
}

#[test]
fn kill_9_before_the_effectful_publish_resumes_with_an_identical_event_sequence() {
    let (clean, resumed) = kill_at(7, "pre"); // authority committed, publish not yet
    same_outcome(&clean, &resumed);
    assert_eq!(clean["run"]["attempt"], 1);
    assert_eq!(resumed["run"]["attempt"], 2, "the second claim finished the job");
}

#[test]
fn kill_9_right_after_the_publish_commit_does_not_publish_again() {
    let (clean, resumed) = kill_at(8, "post");
    same_outcome(&clean, &resumed);
    assert_eq!(resumed["events"].as_array().unwrap().iter().filter(|e| e.as_str().unwrap().starts_with("thread:publish")).count(), 1);
}

#[test]
fn kill_9_mid_pipeline_resumes_too() {
    let (clean, resumed) = kill_at(2, "mid");
    same_outcome(&clean, &resumed);
}

/// Kill INSIDE the publish effect: the effect ran (ledger line) but its commit did not. The engine records the effect
/// intent before running it, so the resumed attempt must NOT blindly re-run it: it stops with NeedsReconciliation(8).
/// No second publish effect, no publish event, no successor (a human reconciles; this slice does not).
#[test]
fn kill_9_inside_the_publish_effect_before_its_commit_needs_reconciliation_and_never_republishes() {
    let work = tmp("mid-commit");
    let marker = work.with_extension("marker");
    let ledger = work.with_extension("ledger");
    let _ = std::fs::remove_file(&marker);
    let _ = std::fs::remove_file(&ledger);
    let mut child = cmd(&work, "1000").arg("--ledger").arg(&ledger).arg("--kill-in-publish").arg(&marker).stdout(Stdio::null()).spawn().unwrap();
    for _ in 0..1500 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists(), "the publish effect never ran");
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success(), "killed, not a clean exit");
    assert_eq!(std::fs::read_to_string(&ledger).unwrap().lines().count(), 1, "the effect ran once before the kill");
    let out = cmd(&work, "1060").arg("--ledger").arg(&ledger).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "the resumed job stops, it does not re-publish");
    assert!(String::from_utf8_lossy(&out.stderr).contains("NeedsReconciliation(8)"), "{}", String::from_utf8_lossy(&out.stderr));
    let resumed: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(std::fs::read_to_string(&ledger).unwrap().lines().count(), 1, "the effect was not run a second time");
    assert!(resumed["events"].as_array().unwrap().iter().all(|e| !e.as_str().unwrap().starts_with("thread:publish")), "no publish commit");
    assert_eq!(resumed["successor"], Value::Null, "no successor without a committed publish");
}

#[test]
fn a_clean_run_and_a_kill_after_the_publish_commit_invoke_the_publish_effect_once() {
    let work = tmp("ledger-post");
    let marker = work.with_extension("marker");
    let ledger = work.with_extension("ledger");
    let _ = std::fs::remove_file(&marker);
    let _ = std::fs::remove_file(&ledger);
    let mut child = cmd(&work, "1000").arg("--ledger").arg(&ledger).arg("--marker").arg(&marker).arg("8").stdout(Stdio::null()).spawn().unwrap();
    for _ in 0..1500 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists());
    child.kill().unwrap();
    child.wait().unwrap();
    report(&cmd(&work, "1060").arg("--ledger").arg(&ledger).output().unwrap());
    assert_eq!(std::fs::read_to_string(&ledger).unwrap().lines().count(), 1, "no second publish effect after the commit");
}
