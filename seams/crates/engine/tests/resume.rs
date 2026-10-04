use engine::{demo, event_log, run_once, FileStore, Options, RunError};

fn opts(crash_after: Option<usize>) -> Options {
    Options { job_id: "job-1".into(), worker_id: "w1".into(), crash_after, after_commit: None }
}
fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("e1-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn kill_between_handlers_loses_an_input() {
    let dir = tmp("lose");
    let store = FileStore::open(&dir).unwrap();
    assert_eq!(run_once(&store, &demo::handlers(), "x", &opts(Some(0))), Err(RunError::Crashed(0)));
    // fresh view of the same store, as a restarted process would have
    let store2 = FileStore::open(&dir).unwrap();
    assert_eq!(run_once(&store2, &demo::handlers(), "x", &opts(None)).unwrap(), "xabc");
    let s3 = FileStore::open(tmp("clean")).unwrap();
    run_once(&s3, &demo::handlers(), "x", &opts(None)).unwrap();
    assert_eq!(event_log(&store2, 3).unwrap(), event_log(&s3, 3).unwrap());
}

#[test]
fn kill_9_between_two_handlers_resumes_with_identical_event_sequence() {
    use std::process::{Command, Stdio};
    let exe = env!("CARGO_BIN_EXE_run_once");
    let clean = tmp("proc-clean");
    let base = Command::new(exe).arg(&clean).output().unwrap();
    assert!(base.status.success());

    let dir = tmp("proc-kill");
    let marker = dir.with_extension("marker");
    let _ = std::fs::remove_file(&marker);
    let mut child = Command::new(exe)
        .arg(&dir).arg(&marker).arg("0")
        .stdout(Stdio::null()).spawn().unwrap();
    for _ in 0..500 {
        if marker.exists() { break; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists(), "handler 0 never committed");
    child.kill().unwrap(); // hard kill, no cleanup
    child.wait().unwrap();

    let resumed = Command::new(exe).arg(&dir).output().unwrap();
    assert!(resumed.status.success());
    assert_eq!(resumed.stdout, base.stdout);
    assert!(!resumed.stdout.is_empty());
}
