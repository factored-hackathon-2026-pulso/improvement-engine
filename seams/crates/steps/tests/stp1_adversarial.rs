mod stp1_common;
use std::sync::Mutex;
use steps::sensor::json;
use steps::{StepError, recompute, sensor};
use stp1_common::*;

static ENV: Mutex<()> = Mutex::new(());

#[test]
fn json_deep_nesting_is_error_not_overflow() {
    let s = "[".repeat(100_000) + &"]".repeat(100_000);
    assert!(json::parse(&s).is_err());
}

#[test]
fn json_strict_numbers_and_duplicate_keys() {
    for bad in ["+1", "01", "1.", "-", ".5", r#"{"a":1,"a":2}"#, "\"a\u{1}b\"", r#""\ud800""#] {
        assert!(json::parse(bad).is_err(), "{bad:?}");
    }
    for ok in ["-0", "0.0", "1e2", "-1.5E-3", r#""\u00e9\ud83d\ude00""#] {
        assert!(json::parse(ok).is_ok(), "{ok:?}");
    }
}

#[test]
fn dot_dot_refs_are_rejected_and_errors_do_not_echo_paths() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { std::env::set_var("STEPS_LAB_DIR", std::env::temp_dir().join("secret-lab-dir")) };
    let input = read_fixture("recompute.in.json");
    let dd = input.replace("lab:sample-1@1", "lab:..@1");
    assert!(matches!(recompute::run(&dd), Err(StepError::Invalid(_))));
    let e = recompute::run(&input.replace("sample-1", "nope-9")).unwrap_err().to_string();
    assert!(!e.contains("secret-lab-dir"), "{e}");
}

#[test]
fn sensor_hung_runner_times_out() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let d = std::env::temp_dir().join(format!("stp1-hang-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let bat = d.join("hang.cmd");
    std::fs::write(&bat, "@echo off\r\nping -n 30 127.0.0.1 >nul\r\n").unwrap();
    unsafe { std::env::set_var("STEPS_RUNNER_EXE", &bat) };
    unsafe { std::env::set_var("STEPS_SNAPSHOT_ROOT", std::env::temp_dir()) };
    unsafe { std::env::set_var("STEPS_RUNNER_TIMEOUT_SECS", "1") };
    let t = std::time::Instant::now();
    let r = sensor::run(&read_fixture("sensors.in.json"));
    unsafe { std::env::remove_var("STEPS_RUNNER_TIMEOUT_SECS") };
    assert!(matches!(r, Err(StepError::Runner(_))));
    assert!(t.elapsed().as_secs() < 15);
}
