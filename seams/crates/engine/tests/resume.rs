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

use engine::JobStore;

#[test]
fn store_rejects_traversal_and_odd_keys() {
    let s = FileStore::open(tmp("keys")).unwrap();
    for k in ["../x", "a\\b", "c:x", "", "/abs", "a/../b", "a b"] {
        assert!(s.cas(k, 0, "v").is_err(), "key {k:?} accepted");
        assert!(s.get(k).is_err(), "key {k:?} read");
    }
}

#[test]
fn corrupt_fence_is_an_error_not_a_silent_reset() {
    let dir = tmp("corrupt");
    let s = FileStore::open(&dir).unwrap();
    run_once(&s, &demo::handlers(), "x", &opts(None)).unwrap();
    std::fs::write(dir.join("kv_fence"), "garbage").unwrap();
    assert!(matches!(run_once(&s, &demo::handlers(), "x", &opts(None)), Err(RunError::Store(_))));
}

#[test]
fn cas_has_exactly_one_winner_under_thread_race() {
    let dir = tmp("race");
    let wins = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|sc| {
        for t in 0..8 {
            let (dir, wins) = (&dir, &wins);
            sc.spawn(move || {
                let s = FileStore::open(dir).unwrap();
                if s.cas("out/0", 0, &format!("v{t}")).is_ok() {
                    wins.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }
    });
    assert_eq!(wins.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn stale_fence_cannot_commit() {
    let dir = tmp("stale");
    let s = FileStore::open(&dir).unwrap();
    struct Bump(std::path::PathBuf);
    impl abi::JobHandler for Bump {
        fn id(&self) -> abi::HandlerId { abi::HandlerId("bump".into()) }
        fn run(&self, _f: &abi::Fence, i: &abi::InputEnvelope) -> Result<abi::OutputEnvelope, abi::HandlerError> {
            // a newer worker claims while this one is mid-handler
            let st = FileStore::open(&self.0).unwrap();
            let (v, a) = st.get("fence").unwrap().unwrap();
            st.cas("fence", v, &(a.parse::<u64>().unwrap() + 1).to_string()).unwrap();
            Ok(abi::OutputEnvelope { payload: i.payload.clone(), events: vec![], effect: abi::EffectState::NoEffect })
        }
    }
    let hs: Vec<Box<dyn abi::JobHandler>> = vec![Box::new(Bump(dir.clone()))];
    assert_eq!(run_once(&s, &hs, "x", &opts(None)), Err(RunError::Handler(abi::HandlerError::StaleFence)));
    assert!(s.get("out/0").unwrap().is_none());
}

#[test]
fn multiline_payload_or_event_is_rejected_not_corrupting() {
    let s = FileStore::open(tmp("nl")).unwrap();
    assert!(matches!(run_once(&s, &demo::handlers(), "x\nE forged", &opts(None)), Err(RunError::Handler(abi::HandlerError::Invalid(_)))));
}
