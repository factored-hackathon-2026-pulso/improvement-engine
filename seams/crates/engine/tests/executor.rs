//! E2 executor: lease/fence per FRZ0 C-7, effect-state enforcement, golden sequence incl. kill -9.
use abi::*;
use engine::executor::{execute, read_lease, ExecError, ExecOptions};
use engine::{demo, event_log, FileStore, JobStore};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

const GOLDEN: &str = include_str!("golden/demo3.events");

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("e2-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}
fn opts(worker: &str, now: u64) -> ExecOptions {
    ExecOptions::new("job-1", worker, now)
}

#[test]
fn executor_fails_a_3_handler_golden_sequence() {
    let s = FileStore::open(tmp("golden")).unwrap();
    assert_eq!(execute(&s, &demo::handlers(), "x", &opts("w1", 1000)).unwrap(), "xabc");
    let log = event_log(&s, 3).unwrap().join("\n") + "\n";
    assert_eq!(log, GOLDEN);
}

#[test]
fn kill_9_between_handlers_reproduces_the_golden_sequence() {
    use std::process::{Command, Stdio};
    let exe = env!("CARGO_BIN_EXE_engine_run");
    let events = |out: &[u8]| -> String {
        String::from_utf8_lossy(out).lines().filter(|l| !l.starts_with("SUMMARY ")).map(|l| format!("{l}\n")).collect()
    };
    let clean = Command::new(exe).args(["demo3"]).arg(tmp("p-clean")).env("ENGINE_NOW", "1000").output().unwrap();
    assert!(clean.status.success());
    assert_eq!(events(&clean.stdout), GOLDEN);

    let dir = tmp("p-kill");
    let marker = dir.with_extension("marker");
    let _ = std::fs::remove_file(&marker);
    let mut child = Command::new(exe)
        .args(["demo3"]).arg(&dir).arg(&marker).arg("1")
        .env("ENGINE_NOW", "1000").stdout(Stdio::null()).spawn().unwrap();
    for _ in 0..500 {
        if marker.exists() { break; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists(), "handler 1 never committed");
    child.kill().unwrap();
    child.wait().unwrap();
    // lease (60s from 1000) is still live: an immediate restart must be refused
    let early = Command::new(exe).args(["demo3"]).arg(&dir).env("ENGINE_NOW", "1059").output().unwrap();
    assert!(!early.status.success());
    // after expiry the job is reclaimed and finishes with the identical sequence
    let resumed = Command::new(exe).args(["demo3"]).arg(&dir).env("ENGINE_NOW", "1060").output().unwrap();
    assert!(resumed.status.success(), "{}", String::from_utf8_lossy(&resumed.stderr));
    assert_eq!(events(&resumed.stdout), GOLDEN);
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("\"attempt\":2"));
}

#[test]
fn lease_is_reclaimable_exactly_at_expiry_with_new_fence_and_attempt() {
    let s = FileStore::open(tmp("lease")).unwrap();
    let mut o = opts("w1", 100);
    o.crash_after = Some(0);
    assert_eq!(execute(&s, &demo::handlers(), "x", &o), Err(ExecError::Crashed(0)));
    let l1 = read_lease(&s).unwrap().unwrap();
    assert_eq!((l1.fence_token, l1.attempt, l1.expires_at), (1, 1, 160));
    assert_eq!(execute(&s, &demo::handlers(), "x", &opts("w2", 159)), Err(ExecError::LeaseHeld { expires_at: 160 }));
    assert_eq!(execute(&s, &demo::handlers(), "x", &opts("w2", 160)).unwrap(), "xabc");
    let l2 = read_lease(&s).unwrap().unwrap();
    assert_eq!((l2.worker_id.as_str(), l2.fence_token, l2.attempt), ("w2", 2, 2));
}

/// Handler that, mid-run, lets a second worker reclaim (after expiry) and finish the job.
struct Usurped(std::path::PathBuf);
impl JobHandler for Usurped {
    fn id(&self) -> HandlerId { HandlerId("usurped".into()) }
    fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        let b = FileStore::open(&self.0).unwrap();
        let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(demo::Append("B"))];
        execute(&b, &hs, "x", &opts("w2", 500)).unwrap();
        Ok(OutputEnvelope { payload: format!("{}A", i.payload), events: vec!["from-a".into()], effect: EffectState::NoEffect })
    }
}

#[test]
fn superseded_worker_cannot_commit() {
    let dir = tmp("stale");
    let s = FileStore::open(&dir).unwrap();
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(Usurped(dir.clone()))];
    assert_eq!(execute(&s, &hs, "x", &opts("w1", 100)), Err(ExecError::StaleFence));
    // out/0 is the new worker's, not the stale one's
    let log = event_log(&s, 1).unwrap();
    assert_eq!(log, vec!["step0:B:x".to_string()]);
}

#[test]
fn worker_past_its_own_lease_cannot_commit_even_if_unclaimed() {
    let s = FileStore::open(tmp("expired")).unwrap();
    let clock = Arc::new(AtomicU64::new(100));
    struct Slow(Arc<AtomicU64>);
    impl JobHandler for Slow {
        fn id(&self) -> HandlerId { HandlerId("slow".into()) }
        fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
            self.0.store(160, Ordering::SeqCst); // now >= expires
            Ok(OutputEnvelope { payload: i.payload.clone(), events: vec![], effect: EffectState::NoEffect })
        }
    }
    let c = clock.clone();
    let mut o = opts("w1", 0);
    o.now = Box::new(move || c.load(Ordering::SeqCst));
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(Slow(clock))];
    assert_eq!(execute(&s, &hs, "x", &o), Err(ExecError::StaleFence));
    assert!(s.get("out/0").unwrap().is_none());
}

struct Eff { runs: Arc<AtomicUsize>, fail: bool }
impl JobHandler for Eff {
    fn id(&self) -> HandlerId { HandlerId("eff".into()) }
    fn effectful(&self) -> bool { true }
    fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if self.fail { return Err(HandlerError::Failed("dispatch lost".into())); }
        Ok(OutputEnvelope { payload: i.payload.clone(), events: vec!["applied".into()], effect: EffectState::AppliedAcknowledged })
    }
}

#[test]
fn committed_effect_is_skipped_on_resume() {
    let s = FileStore::open(tmp("eff-ok")).unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let hs = || -> Vec<Box<dyn JobHandler>> { vec![Box::new(Eff { runs: runs.clone(), fail: false }), Box::new(demo::Append("z"))] };
    let mut o = opts("w1", 0);
    o.crash_after = Some(0);
    assert_eq!(execute(&s, &hs(), "x", &o), Err(ExecError::Crashed(0)));
    assert_eq!(execute(&s, &hs(), "x", &opts("w2", 60)).unwrap(), "xz");
    assert_eq!(runs.load(Ordering::SeqCst), 1, "effectful handler re-ran on resume");
    assert_eq!(event_log(&s, 2).unwrap(), vec!["applied".to_string(), "step1:z:x".to_string()]);
}

#[test]
fn uncommitted_effect_dispatch_blocks_resume_for_reconciliation() {
    let s = FileStore::open(tmp("eff-unknown")).unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(Eff { runs: runs.clone(), fail: true })];
    assert_eq!(execute(&s, &hs, "x", &opts("w1", 0)), Err(ExecError::Handler(HandlerError::Failed("dispatch lost".into()))));
    assert_eq!(execute(&s, &hs, "x", &opts("w2", 60)), Err(ExecError::NeedsReconciliation(0)));
    assert_eq!(runs.load(Ordering::SeqCst), 1, "unknown effect must never be re-dispatched");
}

#[test]
fn failed_pure_handler_is_retried_on_resume() {
    struct Flaky(Arc<AtomicUsize>);
    impl JobHandler for Flaky {
        fn id(&self) -> HandlerId { HandlerId("flaky".into()) }
        fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 { return Err(HandlerError::Failed("transient".into())); }
            Ok(OutputEnvelope { payload: i.payload.clone(), events: vec![], effect: EffectState::NoEffect })
        }
    }
    let s = FileStore::open(tmp("flaky")).unwrap();
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(Flaky(Arc::new(AtomicUsize::new(0))))];
    assert!(execute(&s, &hs, "x", &opts("w1", 0)).is_err());
    assert_eq!(execute(&s, &hs, "x", &opts("w2", 60)).unwrap(), "x");
}

#[test]
fn pure_handler_that_always_fails_stops_after_max_attempts() {
    struct Bad;
    impl JobHandler for Bad {
        fn id(&self) -> HandlerId { HandlerId("bad".into()) }
        fn run(&self, _f: &Fence, _i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
            Err(HandlerError::Failed("always".into()))
        }
    }
    let s = FileStore::open(tmp("maxattempts")).unwrap();
    let hs: Vec<Box<dyn JobHandler>> = vec![Box::new(Bad)];
    for k in 0..3u64 {
        let mut o = opts("w", k * 60);
        o.max_attempts = 3;
        assert!(matches!(execute(&s, &hs, "x", &o), Err(ExecError::Handler(_))));
    }
    let mut o = opts("w", 3 * 60);
    o.max_attempts = 3;
    assert_eq!(execute(&s, &hs, "x", &o), Err(ExecError::AttemptsExhausted(3)));
}
