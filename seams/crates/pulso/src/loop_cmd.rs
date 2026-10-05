//! `pulso loop`: ONE run of the improvement loop as a job of the engine host (a systemd timer or `docker compose run --rm pulso-loop`).
//!
//! cells (the S3-synced inputs mirror) -> sensor -> reasoning (Scout, Verifier, Builder, deterministic recompute, compile) -> regression
//! proof on agent-core -> registry writer (the `builder` principal; credentials minted from the service seed) -> announce to the support
//! platform. It is `ValueLoop::run_as` of `pulso run` behind a process of its own: the engine HTTP service is not started and no job store
//! is needed. The engine never approves, publishes or promotes.
//!
//! Environment contract (names only; docs/dev/ENGINE_PROD.md is the full table):
//! - inputs: `PULSO_LOOP_INPUTS_DIR` (+ `PULSO_LOOP_CELLS_FILE`, default `cells.ndjson`) or an explicit `PULSO_CELLS_NDJSON`; `PULSO_CELLS_SOURCE`.
//! - work: `PULSO_WORK_DIR` (lock, per-finding records, receipts, proofs, run results).
//! - core: `PULSO_REGISTRY_ADDR` (falls back to `PULSO_CORE_ADDR`), `PULSO_SERVICE_SEED_HEX` + `PULSO_SERVICE_KID` (or `PULSO_REGISTRY_TOKEN`).
//! - everything else is the value loop's (`PULSO_LLM_GATEWAY*`, `PULSO_PROFILE`, `PULSO_LOOP_MAX_*`, `PULSO_ANNOUNCE_TO_PLATFORM`, ...).
//!
//! Exit codes (systemd): 0 the run finished (every finding has a closed outcome, blocked or denied included); 1 the run could not run
//! (unreadable cells, sensor, model setup: retry); 2 refused configuration (never retry as is); 3 the run finished but a finding ended on
//! an infrastructure failure (registry, evaluation, delivery: its record is kept and a re-run resumes it); 75 another run holds the lock
//! (nothing was done; `SuccessExitStatus=75`).
use crate::run::log::Logger;
use crate::run::value_loop::{Persist, ValueLoop, default_script_dir};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_REFUSED: i32 = 2;
pub const EXIT_INFRA_FINDING: i32 = 3;
pub const EXIT_LOCKED: i32 = 75;

pub const USAGE: &str = "usage: pulso loop [--check] [--break-lock]\nOne run of the improvement loop (cells -> findings -> reasoning -> proof -> registry -> announce). Environment only:\n\
PULSO_LOOP_INPUTS_DIR [+ PULSO_LOOP_CELLS_FILE] | PULSO_CELLS_NDJSON, PULSO_CELLS_SOURCE, PULSO_WORK_DIR, PULSO_REGISTRY_ADDR | PULSO_CORE_ADDR,\n\
PULSO_SERVICE_SEED_HEX + PULSO_SERVICE_KID | PULSO_REGISTRY_TOKEN, PULSO_PROFILE=standard|demo, PULSO_LOOP_LOCK_TTL_S, PULSO_LOOP_RUN_ID, plus the value-loop variables.\n\
--check validates the configuration and the inputs, prints one JSON line and exits (0 ok, 2 refused) without running anything.\n\
--break-lock removes a lock left by a killed run, then runs.\n\
Exit: 0 finished, 1 could not run, 2 refused config, 3 finished with an infrastructure failure on a finding, 75 another run holds the lock.";

pub const LOCK_FILE: &str = "loop.lock";
pub const DEFAULT_LOCK_TTL_S: u64 = 7200;

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The environment the loop reads, with the two driver conveniences: the cells path from the inputs mirror and the registry address from
/// the Core address infra renders. A variable that is set always wins.
pub fn driver_lookup<'a>(get: &'a dyn Fn(&str) -> Option<String>) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| {
        let own = get(k).filter(|v| !v.is_empty());
        if own.is_some() {
            return own;
        }
        match k {
            "PULSO_CELLS_NDJSON" => {
                let dir = get("PULSO_LOOP_INPUTS_DIR").filter(|v| !v.is_empty())?;
                let file = get("PULSO_LOOP_CELLS_FILE").filter(|v| !v.is_empty()).unwrap_or_else(|| "cells.ndjson".into());
                Some(Path::new(&dir).join(file).to_string_lossy().into_owned())
            }
            "PULSO_REGISTRY_ADDR" => get("PULSO_CORE_ADDR").filter(|v| !v.is_empty()),
            _ => None,
        }
    }
}

/// Exclusive run lock: `loop.lock` created with `create_new` in the work dir; released on drop. A lock older than the TTL (a killed run
/// leaves one behind) is taken over. Content: pid and start time only.
pub struct RunLock {
    path: PathBuf,
}

pub enum LockError {
    Busy { age_s: u64 },
    Io(String),
}

impl RunLock {
    pub fn acquire(work: &Path, ttl_s: u64, now: u64) -> Result<RunLock, LockError> {
        let path = work.join(LOCK_FILE);
        for attempt in 0..2 {
            match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", json!({"pid": std::process::id(), "started_unix": now}));
                    return Ok(RunLock { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let age = fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| now.saturating_sub(d.as_secs()));
                    if attempt == 0 && age > ttl_s {
                        let _ = fs::remove_file(&path); // stale: the run that held it is long gone
                        continue;
                    }
                    return Err(LockError::Busy { age_s: age });
                }
                Err(e) => return Err(LockError::Io(e.kind().to_string())),
            }
        }
        Err(LockError::Busy { age_s: 0 })
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Per-finding records on disk (`<work>/loop-store/<step>.json`): a re-run over the same findings reuses what it already committed.
pub struct FilePersist {
    dir: PathBuf,
}

impl FilePersist {
    pub fn open(work: &Path) -> Result<FilePersist, String> {
        let dir = work.join("loop-store");
        fs::create_dir_all(&dir).map_err(|e| format!("loop store: {}", e.kind()))?;
        Ok(FilePersist { dir })
    }
}

impl Persist for FilePersist {
    fn get(&self, step: u32) -> Option<String> {
        fs::read_to_string(self.dir.join(format!("{step}.json"))).ok()
    }
    fn put(&self, step: u32, record: &str) -> Result<(), String> {
        let tmp = self.dir.join(format!("{step}.json.tmp"));
        fs::write(&tmp, record).and_then(|()| fs::rename(&tmp, self.dir.join(format!("{step}.json")))).map_err(|e| format!("loop store: {}", e.kind()))
    }
}

/// Delivery reasons, proof verdicts and a model that was not there: what says "the infrastructure failed", not "the finding was refused".
const INFRA_REASONS: &[&str] = &["registry_unreachable", "outcome_unknown", "registry_error", "readback_unavailable", "unauthorized"];

/// How many findings of a finished run ended on an infrastructure failure.
pub fn infra_failures(out: &Value) -> usize {
    out["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| {
            let reason = r["delivery"]["reason"].as_str().unwrap_or("");
            let outcome = r["outcome"].as_str().unwrap_or("");
            INFRA_REASONS.contains(&reason)
                || (r["status"] == "blocked" && r["reason"] == "model_unavailable")
                || r["evaluation"]["verdict"] == "failed_infra"
                || outcome.contains("failed_infra")
                || outcome.ends_with(":suite_error")
                || outcome.strip_prefix("proven_not_delivered:").is_some_and(|x| INFRA_REASONS.contains(&x))
        })
        .count()
}

/// The proof runs two Python scripts (`build_suite.py`, `judge_story.py`; PyYAML): without them the proof cannot run, and a proposal that is not
/// proven is never announced. Checked up front so the job refuses with the reason instead of failing at the first finding.
fn proof_prerequisites(get: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
    let python = get("PULSO_REGRESSION_PYTHON").unwrap_or_else(|| "python".into());
    let mut parts = python.split_whitespace();
    let exe = parts.next().ok_or("PULSO_REGRESSION_PYTHON is blank")?;
    let ok = std::process::Command::new(exe).args(parts).arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
    if !ok {
        return Err("the regression proof needs Python (PULSO_REGRESSION_PYTHON) and it did not start: use an image with python and scripts/regression, or set PULSO_EVAL_BEFORE_ANNOUNCE=off (proposals are then delivered unproven and not announced)".into());
    }
    let dir = get("PULSO_REGRESSION_SCRIPTS").map(PathBuf::from).unwrap_or_else(default_script_dir);
    for f in ["build_suite.py", "judge_story.py"] {
        if !dir.join(f).is_file() {
            return Err(format!("the regression proof needs scripts/regression/{f} (PULSO_REGRESSION_SCRIPTS): not found"));
        }
    }
    Ok(())
}

struct Prepared {
    value_loop: ValueLoop,
    work: PathBuf,
    run_id: String,
    lock_ttl_s: u64,
}

fn prepare(get: &dyn Fn(&str) -> Option<String>) -> Result<Prepared, String> {
    let get = driver_lookup(get);
    let work = get("PULSO_WORK_DIR").map(PathBuf::from).ok_or("PULSO_WORK_DIR is required (lock, records, receipts, results)")?;
    if get("PULSO_CELLS_NDJSON").is_none() {
        return Err("PULSO_LOOP_INPUTS_DIR (or PULSO_CELLS_NDJSON) is required: where the S3-synced cells package lives".into());
    }
    let lock_ttl_s = match get("PULSO_LOOP_LOCK_TTL_S") {
        None => DEFAULT_LOCK_TTL_S,
        Some(v) => v.trim().parse::<u64>().ok().filter(|n| *n >= 60).ok_or("PULSO_LOOP_LOCK_TTL_S must be a whole number of seconds, at least 60")?,
    };
    let value_loop = ValueLoop::from_lookup(&get, Some(&work))?.ok_or("PULSO_CELLS_NDJSON is not set")?;
    if value_loop.proof.is_some() {
        proof_prerequisites(&get)?;
    }
    let run_id = get("PULSO_LOOP_RUN_ID").map(|s| s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).take(64).collect::<String>()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("loop-{}", now_secs()));
    Ok(Prepared { value_loop, work, run_id, lock_ttl_s })
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, path)).map_err(|e| format!("{}: {}", path.file_name().and_then(|n| n.to_str()).unwrap_or("result"), e.kind()))
}

/// `pulso loop`. See the module docs for the exit codes.
pub fn main(args: &[String]) -> i32 {
    run(args, &|k| std::env::var(k).ok(), &Logger::stdout())
}

pub fn run(args: &[String], get: &dyn Fn(&str) -> Option<String>, log: &Logger) -> i32 {
    let (mut check, mut break_lock) = (false, false);
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            "--break-lock" => break_lock = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return EXIT_OK;
            }
            other => {
                eprintln!("pulso loop: unknown argument {other:?}\n{USAGE}");
                return EXIT_REFUSED;
            }
        }
    }
    let p = match prepare(get) {
        Ok(p) => p,
        Err(e) => {
            log.error("loop_refused", json!({"reason": e}));
            eprintln!("pulso loop: refusing to start: {e}");
            return EXIT_REFUSED;
        }
    };
    let cells = p.value_loop.cells.clone();
    if check {
        let readable = cells.is_file();
        log.info("loop_check", json!({"run_id": p.run_id, "cells_present": readable, "support_profile": p.value_loop.profile.as_str(), "source": p.value_loop.source.as_str(),
                                      "auth_mode": p.value_loop.credential, "proof": p.value_loop.proof.is_some(), "announce": p.value_loop.announcer.is_some()}));
        return EXIT_OK;
    }
    if let Err(e) = fs::create_dir_all(&p.work) {
        log.error("loop_failed", json!({"reason": format!("work dir: {}", e.kind())}));
        return EXIT_FAILED;
    }
    if break_lock {
        let _ = fs::remove_file(p.work.join(LOCK_FILE));
    }
    let _lock = match RunLock::acquire(&p.work, p.lock_ttl_s, now_secs()) {
        Ok(l) => l,
        Err(LockError::Busy { age_s }) => {
            log.warn("loop_locked", json!({"run_id": p.run_id, "lock_age_s": age_s, "action": "skipped: another run holds the lock"}));
            return EXIT_LOCKED;
        }
        Err(LockError::Io(k)) => {
            log.error("loop_failed", json!({"reason": format!("lock: {k}")}));
            return EXIT_FAILED;
        }
    };
    if !cells.is_file() {
        log.error("loop_failed", json!({"run_id": p.run_id, "reason": "cells_missing", "hint": "the inputs mirror has not synced the cells package"}));
        return EXIT_FAILED;
    }
    let persist = match FilePersist::open(&p.work) {
        Ok(x) => x,
        Err(e) => {
            log.error("loop_failed", json!({"reason": e}));
            return EXIT_FAILED;
        }
    };
    log.info(
        "loop_start",
        json!({"run_id": p.run_id, "source": p.value_loop.source.as_str(), "support_profile": p.value_loop.profile.as_str(), "auth_mode": p.value_loop.credential,
               "proof": p.value_loop.proof.is_some(), "announce": p.value_loop.announcer.is_some(), "max_findings": p.value_loop.max_findings, "max_exploratory": p.value_loop.max_exploratory}),
    );
    let started = now_secs();
    let out = match p.value_loop.run_as(&persist, &format!("value-loop-{}", p.run_id)) {
        Ok(o) => o,
        Err(e) => {
            log.error("loop_failed", json!({"run_id": p.run_id, "reason": e}));
            return EXIT_FAILED;
        }
    };
    for (i, r) in out["findings"].as_array().into_iter().flatten().enumerate() {
        log.info(
            "loop_finding",
            json!({"run_id": p.run_id, "step": i + 1, "metric": r["metric"], "status": r["status"], "reason": r["reason"], "outcome": r["outcome"], "delivery": r["delivery"]["status"],
                   "delivery_reason": r["delivery"]["reason"], "resumed": r["resumed_from_job_store"]}),
        );
    }
    let infra = infra_failures(&out);
    let results = p.work.join("loop-runs");
    let saved = fs::create_dir_all(&results).map_err(|e| e.kind().to_string()).and_then(|()| {
        let text = out.to_string();
        write_atomic(&results.join(format!("{}.json", p.run_id)), &text)?;
        write_atomic(&p.work.join("loop-latest.json"), &text)
    });
    if let Err(e) = saved {
        log.error("loop_failed", json!({"run_id": p.run_id, "reason": format!("results not written: {e}")}));
        return EXIT_FAILED;
    }
    log.info("loop_done", json!({"run_id": p.run_id, "duration_s": now_secs().saturating_sub(started), "summary": out["summary"], "support_profile": out["support_profile"], "infra_failures": infra}));
    if infra > 0 { EXIT_INFRA_FINDING } else { EXIT_OK }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pulso-loopcmd-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn lk(v: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let m: HashMap<String, String> = v.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn the_cells_path_comes_from_the_inputs_mirror_and_an_explicit_path_wins() {
        let g = lk(&[("PULSO_LOOP_INPUTS_DIR", "/var/lib/pulso/inputs")]);
        let d = driver_lookup(&g);
        assert!(d("PULSO_CELLS_NDJSON").unwrap().replace('\\', "/").ends_with("/var/lib/pulso/inputs/cells.ndjson"));
        let g = lk(&[("PULSO_LOOP_INPUTS_DIR", "/in"), ("PULSO_LOOP_CELLS_FILE", "bank.ndjson")]);
        assert!(driver_lookup(&g)("PULSO_CELLS_NDJSON").unwrap().replace('\\', "/").ends_with("/in/bank.ndjson"));
        let g = lk(&[("PULSO_LOOP_INPUTS_DIR", "/in"), ("PULSO_CELLS_NDJSON", "/explicit/c.ndjson")]);
        assert_eq!(driver_lookup(&g)("PULSO_CELLS_NDJSON").unwrap(), "/explicit/c.ndjson");
        assert!(driver_lookup(&lk(&[]))("PULSO_CELLS_NDJSON").is_none());
    }

    #[test]
    fn the_registry_address_falls_back_to_the_core_address_infra_renders() {
        let g = lk(&[("PULSO_CORE_ADDR", "10.0.1.5:8001")]);
        assert_eq!(driver_lookup(&g)("PULSO_REGISTRY_ADDR").unwrap(), "10.0.1.5:8001");
        let g = lk(&[("PULSO_CORE_ADDR", "10.0.1.5:8001"), ("PULSO_REGISTRY_ADDR", "127.0.0.1:9")]);
        assert_eq!(driver_lookup(&g)("PULSO_REGISTRY_ADDR").unwrap(), "127.0.0.1:9");
    }

    #[test]
    fn two_runs_never_overlap_and_the_lock_is_released_when_the_run_ends() {
        let w = tmp("lock");
        let a = RunLock::acquire(&w, 7200, now_secs()).ok().expect("first run takes the lock");
        assert!(matches!(RunLock::acquire(&w, 7200, now_secs()), Err(LockError::Busy { .. })), "second run is refused");
        drop(a);
        assert!(RunLock::acquire(&w, 7200, now_secs()).is_ok(), "released on drop");
    }

    #[test]
    fn a_lock_left_by_a_killed_run_is_taken_over_only_after_the_ttl() {
        let w = tmp("stale");
        let held = RunLock::acquire(&w, 7200, now_secs()).ok().unwrap();
        std::mem::forget(held); // a killed run: the file stays
        assert!(matches!(RunLock::acquire(&w, 7200, now_secs() + 100), Err(LockError::Busy { .. })), "young lock: busy");
        assert!(RunLock::acquire(&w, 7200, now_secs() + 7201 + 5).is_ok(), "older than the TTL: taken over");
    }

    #[test]
    fn the_file_store_round_trips_and_a_missing_step_is_none() {
        let w = tmp("store");
        let s = FilePersist::open(&w).unwrap();
        assert!(s.get(1).is_none());
        s.put(1, "{\"a\":1}").unwrap();
        assert_eq!(s.get(1).as_deref(), Some("{\"a\":1}"));
        s.put(1, "{\"a\":2}").unwrap();
        assert_eq!(s.get(1).as_deref(), Some("{\"a\":2}"));
    }

    #[test]
    fn infrastructure_failures_are_told_apart_from_closed_refusals() {
        let out = json!({"findings": [
            {"status": "unlinked", "delivery": null},
            {"status": "proposed", "outcome": "announced", "delivery": {"status": "delivered"}},
            {"status": "proposed", "outcome": "not_announced:not_fixed", "delivery": null},
            {"status": "proposed", "outcome": "not_announced:suite_error", "delivery": null},
            {"status": "proposed", "outcome": "proven_not_delivered:registry_unreachable", "delivery": {"status": "denied", "reason": "registry_unreachable"}},
            {"status": "proposed", "outcome": "x", "delivery": {"status": "denied", "reason": "quota_exceeded"}},
            {"status": "proposed", "outcome": "x", "delivery": {"status": "denied", "reason": "forbidden_role"}},
            {"status": "proposed", "outcome": "not_announced:x", "evaluation": {"verdict": "failed_infra"}, "delivery": null},
            {"status": "blocked", "reason": "model_unavailable", "delivery": null},
            {"status": "blocked", "reason": "rubric_reject", "delivery": null}
        ]});
        assert_eq!(infra_failures(&out), 4, "suite_error, registry_unreachable, failed_infra and an unavailable model only");
        assert_eq!(infra_failures(&json!({"findings": []})), 0);
    }

    #[derive(Clone, Default)]
    struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Buf {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    #[test]
    fn a_missing_configuration_is_refused_with_exit_2_naming_the_variable() {
        let w = tmp("refuse");
        let wd = w.to_string_lossy().to_string();
        let cases: Vec<(Vec<(&str, &str)>, &str)> = vec![
            (vec![], "PULSO_WORK_DIR"),
            (vec![("PULSO_WORK_DIR", &wd)], "PULSO_LOOP_INPUTS_DIR"),
            (vec![("PULSO_WORK_DIR", &wd), ("PULSO_LOOP_INPUTS_DIR", "/in")], "PULSO_CELLS_SOURCE"),
            (vec![("PULSO_WORK_DIR", &wd), ("PULSO_LOOP_INPUTS_DIR", "/in"), ("PULSO_CELLS_SOURCE", "synthetic")], "PULSO_REGISTRY_ADDR"),
            (vec![("PULSO_WORK_DIR", &wd), ("PULSO_LOOP_INPUTS_DIR", "/in"), ("PULSO_CELLS_SOURCE", "synthetic"), ("PULSO_CORE_ADDR", "10.0.0.1:8001")], "PULSO_SERVICE_SEED_HEX"),
        ];
        for (env, needle) in cases {
            let buf = Buf::default();
            let log = Logger::new(Box::new(buf.clone()));
            assert_eq!(run(&[], &lk(&env), &log), EXIT_REFUSED, "{needle}");
            let shown = buf.text();
            assert!(shown.contains(needle) && shown.contains("loop_refused"), "{needle}: {shown}");
        }
        assert_eq!(run(&["--nope".to_string()], &lk(&[]), &Logger::new(Box::new(std::io::sink()))), EXIT_REFUSED);
    }

    #[test]
    fn the_demo_profile_is_refused_for_real_data_at_the_driver_too() {
        let w = tmp("demo-refused");
        let log = Logger::new(Box::new(std::io::sink()));
        let base = [("PULSO_WORK_DIR", w.to_str().unwrap()), ("PULSO_LOOP_INPUTS_DIR", "/in"), ("PULSO_CELLS_SOURCE", "bank"), ("PULSO_PROFILE", "demo"), ("PULSO_REGISTRY_ADDR", "127.0.0.1:1"), ("PULSO_REGISTRY_TOKEN", "t"),
                    ("PULSO_LLM_GATEWAY", "enabled"), ("PULSO_LLM_GATEWAY_ADDR", "127.0.0.1:1"), ("PULSO_LLM_GATEWAY_KEY", "k")];
        assert_eq!(run(&["--check".to_string()], &lk(&base), &log), EXIT_REFUSED);
    }
}
