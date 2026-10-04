//! Q1: the ten report steps with host=rust, every label honest (doubles vocabulary), offline.
use serde_json::Value;
use thread10::{Opts, run};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("t10-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

pub fn opts(name: &str, over: bool) -> Opts {
    Opts { work: tmp(name), runner: env!("CARGO_BIN_EXE_synth_runner").into(), human_override: over, denied_kind: false, sha: "0".repeat(40) }
}

fn status(r: &Value, n: u64, id: &str) -> String {
    r["steps"].as_array().unwrap().iter().find(|s| s["n"] == n && s["id"] == id).unwrap_or_else(|| panic!("step {n} {id}"))["status"].as_str().unwrap().to_string()
}

#[test]
fn ten_steps_run_with_host_rust_and_no_step_is_labelled_real() {
    let r = run(&opts("ten", true)).expect("run");
    assert_eq!(r.error, None, "{:?}", r.events);
    let rep = &r.report;
    assert_eq!(rep["host"], "rust");
    let steps = rep["steps"].as_array().unwrap();
    let ns: std::collections::BTreeSet<u64> = steps.iter().map(|s| s["n"].as_u64().unwrap()).collect();
    assert_eq!(ns, (1..=10).collect(), "ten steps");
    assert!(steps.iter().all(|s| s["host"] == "rust" && s["status"] != "real" && s["data_class"] == "generated_sample"), "{steps:?}");
    let want = [(1, "trigger", "stand-in"), (2, "signals", "stand-in"), (3, "scout", "stand-in"), (3, "recompute", "real-narrow"), (4, "opportunity", "stand-in"),
        (4, "validation", "real-narrow"), (5, "compile", "stand-in"), (6, "gate", "stand-in"), (7, "revision", "not_exercised"), (8, "approval", "simulated"),
        (9, "publish", "stand-in"), (10, "observation", "simulated")];
    for (n, id, st) in want {
        assert_eq!(status(rep, n, id), st, "step {n} {id}");
    }
    assert_eq!(rep["gate"]["verdict"], "fail");
    assert_eq!(rep["overrides"][0]["label"], "human_override");
    assert_eq!(rep["overrides"][0]["simulated"], true);
    assert_eq!(rep["quality_claims"], "forbidden");
}
