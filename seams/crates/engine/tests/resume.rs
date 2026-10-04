use engine::{demo, run_once, FileStore, JobStore, Options, RunError};

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
    assert_eq!(store2.events().unwrap(), s3.events().unwrap());
}
