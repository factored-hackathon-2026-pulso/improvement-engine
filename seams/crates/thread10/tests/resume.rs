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
