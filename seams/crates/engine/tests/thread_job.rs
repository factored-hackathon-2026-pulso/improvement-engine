//! E2 step 2: the 5-step `thread` job (signals -> recompute -> validation -> compile -> gate) through the
//! executor on synthetic data. Package/lab files are built by `engine::synth` (no real E0 rows).
use engine::adapters::thread_handlers;
use engine::executor::{execute, ExecError, ExecOptions};
use engine::{event_log, synth, FileStore};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("e2t-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

const EXPECTED: [&str; 5] = [
    "thread:sensors:signals=1,discards=1:semantics=claude-standin",
    "thread:recompute:matches=1/1:semantics=claude-standin",
    "thread:validation:verdict=corroborated:semantics=claude-standin",
    "thread:compile:status=compiled:semantics=claude-standin",
    "thread:gate:verdict=pass:semantics=claude-standin",
];

fn runner() -> std::path::PathBuf {
    env!("CARGO_BIN_EXE_synth_runner").into()
}

#[test]
fn five_step_synthetic_job_runs_through_the_executor() {
    let work = tmp("five");
    let (env, spec) = synth::build(&work, &runner(), None).unwrap();
    let store = FileStore::open(work.join("store")).unwrap();
    let hs = thread_handlers(env, None);
    assert_eq!(hs.len(), 5);
    let fin = execute(&store, &hs, &spec, &ExecOptions::new("thread-1", "w1", 1000)).unwrap();
    assert_eq!(event_log(&store, 5).unwrap(), EXPECTED);
    assert!(fin.contains("\"gate\":{") && fin.contains("\"verdict\":\"pass\""), "{fin}");
    assert!(!fin.contains('\n'));
}

#[test]
fn uncorroborated_signal_blocks_compile_and_keeps_earlier_events() {
    let work = tmp("refuted");
    let (env, spec) = synth::build(&work, &runner(), Some(0.5)).unwrap(); // lab says 0.30
    let store = FileStore::open(work.join("store")).unwrap();
    let hs = thread_handlers(env, None);
    let r = execute(&store, &hs, &spec, &ExecOptions::new("thread-2", "w1", 1000));
    assert!(matches!(&r, Err(ExecError::Handler(abi::HandlerError::Failed(m))) if m.contains("blocked") && m.contains("refuted")), "{r:?}");
    let log = event_log(&store, 5).unwrap();
    assert_eq!(log.len(), 3);
    assert_eq!(log[2], "thread:validation:verdict=refuted:semantics=claude-standin");
}

#[test]
fn kill_9_mid_thread_resumes_with_identical_sequence() {
    use std::process::{Command, Stdio};
    let exe = env!("CARGO_BIN_EXE_engine_run");
    let events = |out: &[u8]| -> Vec<String> {
        String::from_utf8_lossy(out).lines().filter(|l| !l.starts_with("SUMMARY ")).map(String::from).collect()
    };
    let run = |store: &std::path::Path, work: &std::path::Path, now: &str| {
        Command::new(exe).arg("thread").arg(store).arg(work).env("ENGINE_NOW", now)
            .env("STEPS_RUNNER_EXE", runner()).output().unwrap()
    };
    let (cs, cw) = (tmp("k-clean-s"), tmp("k-clean-w"));
    let clean = run(&cs, &cw, "1000");
    assert!(clean.status.success(), "{}", String::from_utf8_lossy(&clean.stderr));
    assert_eq!(events(&clean.stdout), EXPECTED);

    let (ks, kw) = (tmp("k-kill-s"), tmp("k-kill-w"));
    let marker = ks.with_extension("marker");
    let _ = std::fs::remove_file(&marker);
    let mut child = Command::new(exe).arg("thread").arg(&ks).arg(&kw).arg(&marker).arg("2")
        .env("ENGINE_NOW", "1000").env("STEPS_RUNNER_EXE", runner()).stdout(Stdio::null()).spawn().unwrap();
    for _ in 0..1000 {
        if marker.exists() { break; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists(), "handler 2 never committed");
    child.kill().unwrap();
    child.wait().unwrap();
    let resumed = run(&ks, &kw, "1060");
    assert!(resumed.status.success(), "{}", String::from_utf8_lossy(&resumed.stderr));
    assert_eq!(events(&resumed.stdout), EXPECTED);
    let summary = String::from_utf8_lossy(&resumed.stdout).lines().last().unwrap().to_string();
    assert!(summary.contains("\"status\":\"completed\"") && summary.contains("\"attempt\":2"), "{summary}");
}

#[test]
fn live_stubs_report_not_exercised_and_blocked_core_honestly() {
    let work = tmp("live");
    let (env, spec) = synth::build(&work, &runner(), None).unwrap();
    let store = FileStore::open(work.join("store")).unwrap();
    let mut hs = thread_handlers(env, None);
    hs.extend(engine::live::stub_handlers());
    execute(&store, &hs, &spec, &ExecOptions::new("thread-3", "w1", 1000)).unwrap();
    let log = event_log(&store, 7).unwrap();
    assert_eq!(log[5], "thread:arms:not_exercised:needs=core-client(K3)");
    assert_eq!(log[6], "thread:publish:blocked(core):needs=core-client(K3)+INT");
}
