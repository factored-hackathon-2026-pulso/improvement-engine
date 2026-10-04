//! E2 executor: lease/fence per FRZ0 C-7, effect-state enforcement, golden sequence incl. kill -9.
use engine::{FileStore, JobStore};
use std::sync::Arc;

const GOLDEN: &str = engine::conformance::GOLDEN;

fn tmp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("e2-{}-{}", name, std::process::id()))
}
fn tmp(name: &str) -> std::path::PathBuf {
    let d = tmp_path(name);
    let _ = std::fs::remove_dir_all(&d);
    d
}

struct Files;
impl engine::conformance::Backend for Files {
    fn fresh(&self, name: &str) -> Box<dyn JobStore> {
        Box::new(FileStore::open(tmp(name)).unwrap())
    }
    fn reopener(&self, name: &str) -> Arc<dyn Fn() -> Box<dyn JobStore>> {
        let dir = tmp_path(name);
        Arc::new(move || Box::new(FileStore::open(&dir).unwrap()))
    }
}

#[test]
fn executor_suite_on_the_file_store() {
    let f = engine::conformance::run_suite(&Files);
    assert!(f.is_empty(), "{f:#?}");
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

