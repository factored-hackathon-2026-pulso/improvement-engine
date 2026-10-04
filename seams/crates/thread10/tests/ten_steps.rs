//! Q1: the ten report steps with host=rust, every label honest (doubles vocabulary), offline.
use serde_json::Value;
use thread10::{Opts, run};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("t10-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

pub fn opts(name: &str, over: bool) -> Opts {
    Opts { human_override: over, ..Opts::new(tmp(name), env!("CARGO_BIN_EXE_synth_runner").into()) }
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

fn statuses(r: &Value) -> Vec<(u64, String, String)> {
    r["steps"].as_array().unwrap().iter().map(|s| (s["n"].as_u64().unwrap(), s["id"].as_str().unwrap().into(), s["status"].as_str().unwrap().into())).collect()
}

#[test]
fn denied_kind_blocks_compile_and_nothing_after_it_ran() {
    let mut o = opts("denied", true);
    o.denied_kind = true;
    let r = run(&o).expect("run");
    assert!(r.error.is_some(), "the job must stop");
    assert_eq!(status(&r.report, 5, "compile"), "blocked(kind_not_supported)");
    for (n, id) in [(6, "gate"), (7, "revision"), (8, "approval"), (9, "publish"), (10, "observation")] {
        assert_eq!(status(&r.report, n, id), "not_exercised", "{id}");
    }
    assert!(r.report["authors"]["candidate_created_at"].is_null(), "no candidate exists");
    assert!(r.report.get("overrides").is_none());
    assert!(r.events.iter().all(|e| !e.starts_with("thread:publish")), "{:?}", r.events);
}

#[test]
fn failed_gate_without_a_labelled_override_blocks_steps_8_and_9() {
    let r = run(&opts("nooverride", false)).expect("run");
    assert!(r.error.as_deref().is_some_and(|e| e.contains("blocked(gate)")), "{:?}", r.error);
    assert_eq!(r.report["gate"]["verdict"], "fail");
    assert_eq!(status(&r.report, 6, "gate"), "stand-in");
    assert_eq!(status(&r.report, 8, "approval"), "blocked(gate)");
    assert_eq!(status(&r.report, 9, "publish"), "blocked(gate)");
    assert_eq!(status(&r.report, 10, "observation"), "not_exercised");
    assert!(r.report.get("overrides").is_none(), "{:?}", statuses(&r.report));
}

#[test]
fn an_uncorroborated_claim_blocks_compile() {
    let mut o = opts("refuted", true);
    o.claimed_rate = Some(0.5); // the lab says 0.30
    let r = run(&o).expect("run");
    assert_eq!(status(&r.report, 4, "validation"), "real-narrow");
    assert_eq!(status(&r.report, 5, "compile"), "blocked(validation)");
    assert_eq!(status(&r.report, 9, "publish"), "not_exercised");
}
