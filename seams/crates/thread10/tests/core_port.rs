//! The Core behind steps 5, 6 and 9 is a `CorePort` chosen by the caller. The report derives `real-narrow` from
//! `CorePort::is_real()`, never from configuration: the offline double stays `stand-in`.
use core_client::dto::ArmReport;
use engine::live::{ArmCall, CorePort, FrozenInfo, PublishInfo, SuiteInfo};
use serde_json::Value;
use std::rc::Rc;
use thread10::double::DoublePort;
use thread10::{Opts, run};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("t10c-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn status<'a>(r: &'a Value, id: &str) -> &'a str {
    r["steps"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap()["status"].as_str().unwrap()
}

/// TEST FAKE: the offline double that claims to be the real Core, to observe what the report derives from `is_real()`.
/// It is NOT a Core and proves nothing about one; a live window is what proves the real-narrow steps.
struct ClaimsReal(DoublePort);
impl CorePort for ClaimsReal {
    fn is_real(&self) -> bool {
        true
    }
    fn dry_run(&self, ops: &[String]) -> Result<String, String> {
        self.0.dry_run(ops)
    }
    fn freeze(&self, ops: &[String], job_id: &str) -> Result<FrozenInfo, String> {
        self.0.freeze(ops, job_id)
    }
    fn suite(&self, f: &FrozenInfo) -> Result<SuiteInfo, String> {
        self.0.suite(f)
    }
    fn run_arm(&self, f: &FrozenInfo, c: &ArmCall) -> Result<ArmReport, String> {
        self.0.run_arm(f, c)
    }
    fn evaluate(&self, f: &FrozenInfo, j: &str) -> Result<String, String> {
        self.0.evaluate(f, j)
    }
    fn approve(&self, f: &FrozenInfo) -> Result<String, String> {
        self.0.approve(f)
    }
    fn publish(&self, f: &FrozenInfo, k: &str) -> Result<PublishInfo, String> {
        self.0.publish(f, k)
    }
}

fn opts(name: &str) -> Opts {
    Opts { human_override: true, ..Opts::new(tmp(name), env!("CARGO_BIN_EXE_synth_runner").into()) }
}

fn core_port(r: &Value) -> String {
    r["ports"].as_array().unwrap().iter().find(|p| p["port"] == "core").unwrap()["provenance"].as_str().unwrap().to_string()
}

#[test]
fn the_offline_double_keeps_steps_5_6_9_stand_in() {
    let r = run(&opts("double")).unwrap();
    assert_eq!((status(&r.report, "compile"), status(&r.report, "gate"), status(&r.report, "publish")), ("stand-in", "stand-in", "stand-in"));
    assert!(core_port(&r.report).starts_with("offline-double"));
}

#[test]
fn a_port_that_is_real_makes_steps_5_6_9_real_narrow_and_the_human_stays_simulated() {
    let mut o = opts("real");
    o.core = Some(Rc::new(ClaimsReal(DoublePort::default())));
    let r = run(&o).unwrap();
    assert_eq!((status(&r.report, "compile"), status(&r.report, "gate"), status(&r.report, "publish")), ("real-narrow", "real-narrow", "real-narrow"));
    assert_eq!(status(&r.report, "approval"), "simulated", "the approving human is a simulated issuer even against the real Core");
    assert_eq!(status(&r.report, "observation"), "simulated");
    assert!(core_port(&r.report).starts_with("real-core-live"));
    assert!(r.report["steps"].as_array().unwrap().iter().all(|s| s["status"] != "real"), "narrow, never wholly real");
    let gate = r.report["steps"].as_array().unwrap().iter().find(|s| s["id"] == "gate").unwrap();
    assert_eq!(gate["detail"]["judge"], "claude-gsipy", "the judge is still the stand-in");
}
