mod stp1_common;
use std::sync::Mutex;
use steps::{StepError, sensor};
use stp1_common::*;

static ENV: Mutex<()> = Mutex::new(());

/// Fake runner (a .cmd) that copies the fixture result.json into <output>/run1/ like the real sensor.
fn fake_runner() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("stp1-fake-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let bat = d.join("runner.cmd");
    let src = fixture("result.json");
    std::fs::write(
        &bat,
        format!("@echo off\r\nmkdir \"%~9\\run1\"\r\ncopy /y \"{}\" \"%~9\\run1\\result.json\" >nul\r\n", src.display()),
    )
    .unwrap();
    bat
}

#[test]
fn sensor_wraps_runner_and_output_validates() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("STEPS_RUNNER_EXE", fake_runner()) };
    unsafe { std::env::set_var("STEPS_SNAPSHOT_ROOT", std::env::temp_dir()) };
    let out = sensor::run(&read_fixture("sensors.in.json")).unwrap();
    assert_valid("sensors.out", &out);
    assert!(out.contains("\"numerator\":22") && out.contains("\"denominator\":30"), "{out}");
    assert!(out.contains("\"holdout_checked\":true"), "{out}");
    assert!(out.contains("\"metric_id\":\"e0_technical_error_rate\",\"reason\":\"below_k\""), "{out}");
    assert!(!out.contains("sha256:"), "no private digests or pattern refs leak: {out}");
}

#[test]
fn sensor_reports_missing_runner_and_bad_input() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("STEPS_RUNNER_EXE", "Z:/does/not/exist.exe") };
    assert!(matches!(sensor::run(&read_fixture("sensors.in.json")), Err(StepError::Io(_))));
    assert!(matches!(sensor::run("{}"), Err(StepError::Invalid(_))));
    assert!(matches!(sensor::run("not json"), Err(StepError::Invalid(_))));
}

/// Real runner, only when the exe and python (synthetic package builder) exist.
#[test]
fn sensor_real_runner_smoke() {
    let exe = "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe";
    if !std::path::Path::new(exe).exists() {
        eprintln!("skip: real runner absent");
        return;
    }
    let root = std::env::temp_dir().join(format!("stp1-real-{}", std::process::id()));
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../e2e-core/src");
    let code = "import sys;sys.path.insert(0,sys.argv[1]);from claude_standin.ed0_detect import write_synthetic_e0 as w;w(sys.argv[2],{'A':'qa','B':'qb'})";
    let pkg = root.join("sample-1");
    let st = std::process::Command::new("python")
        .args(["-c", code, src.to_str().unwrap(), pkg.to_str().unwrap()])
        .status();
    if !matches!(st, Ok(s) if s.success()) {
        eprintln!("skip: python package builder unavailable");
        return;
    }
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("STEPS_RUNNER_EXE", exe) };
    unsafe { std::env::set_var("STEPS_SNAPSHOT_ROOT", &root) };
    let out = sensor::run(&read_fixture("sensors.in.json")).unwrap();
    assert_valid("sensors.out", &out);
    assert!(out.contains("\"numerator\":22"), "{out}");
}
