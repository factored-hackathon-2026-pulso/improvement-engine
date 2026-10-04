//! Fixture-based golden capture of the six evaluate outcomes. "Captured N of 6" is COMPUTED from the recorded fixtures
//! `tests/fixtures/v1/<outcome>.json`; an outcome without a valid recorded real (or labelled fault-injected) observation
//! is `NotCaptured` with the reason (never fabricated). The bridge-contract goldens are references, not captures.
//!
//! Fixture: `{outcome, provenance{stack_image, agent_core_sha, date, command, label}, observation, ...evidence}`;
//! `label` is `real` or `fault-injected`; a lost result is always `fault-injected`.
use crate::outcomes::{ArmRollup, EvaluateOutcome, classify_http, classify_observation, rollup_arms};
use core_client::dto::ArmReport;
use serde_json::Value;
use std::path::Path;

pub const FIXTURE_DIR: &str = "tests/fixtures/v1";

const WRITER_EVALUATION: &str = include_str!("../../../../bridge-contract/examples/flows/writer_evaluation.json");
const ARMS: &str = include_str!("../../../../bridge-contract/examples/flows/arms.json");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureStatus {
    Captured { source: String },
    NotCaptured { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub outcome: EvaluateOutcome,
    pub status: CaptureStatus,
}

/// A contract golden that documents a shape. Never counted as a capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldenReference {
    pub outcome: EvaluateOutcome,
    pub source: String,
}

fn step_body(flow: &str, case: &str) -> Option<Value> {
    let d: Value = serde_json::from_str(flow).ok()?;
    d["steps"].as_array()?.iter().find(|s| s["case"] == case).map(|s| s["response"]["body"].clone())
}

pub fn golden_references() -> Vec<GoldenReference> {
    let mut out = vec![];
    if let Some(b) = step_body(WRITER_EVALUATION, "invoke_evaluate_only") {
        let ne = &b["result"]["facts"]["pulso_writer_receipts"]["value"]["native_evaluation"];
        if classify_http(200, ne) == Ok(EvaluateOutcome::Pass) {
            out.push(GoldenReference { outcome: EvaluateOutcome::Pass, source: "bridge-contract/examples/flows/writer_evaluation.json#invoke_evaluate_only (synthetic)".into() });
        }
    }
    if let Some(b) = step_body(ARMS, "arm_budget_unknown")
        && let Ok(r) = ArmReport::from_json(&b)
        && rollup_arms(&[r]) == ArmRollup::Outcome(EvaluateOutcome::FailedInfra)
    {
        out.push(GoldenReference { outcome: EvaluateOutcome::FailedInfra, source: "bridge-contract/examples/flows/arms.json#arm_budget_unknown (arm-level, synthetic)".into() });
    }
    out
}

const PROVENANCE: [&str; 5] = ["stack_image", "agent_core_sha", "date", "command", "label"];

fn judge(o: EvaluateOutcome, doc: &Value) -> Result<String, String> {
    if doc["outcome"].as_str() != Some(o.as_str()) {
        return Err(format!("fixture claims outcome {:?}, expected {:?}", doc["outcome"], o.as_str()));
    }
    let p = &doc["provenance"];
    for f in PROVENANCE {
        if p.get(f).and_then(Value::as_str).is_none_or(str::is_empty) {
            return Err(format!("provenance.{f} is missing"));
        }
    }
    let label = p["label"].as_str().unwrap_or_default();
    if label != "real" && label != "fault-injected" {
        return Err(format!("provenance.label {label:?} is not real or fault-injected"));
    }
    if o == EvaluateOutcome::ResultLost && label != "fault-injected" {
        return Err("a lost result must be labelled fault-injected (the timeout is caused by the fault)".into());
    }
    let got = classify_observation(&doc["observation"])?;
    if got != o {
        return Err(format!("the recorded observation classifies as {:?}, not {:?}", got.as_str(), o.as_str()));
    }
    Ok(format!("{FIXTURE_DIR}/{}.json ({label}, image {}, {})", o.as_str(), p["stack_image"].as_str().unwrap_or(""), p["date"].as_str().unwrap_or("")))
}

fn missing(o: EvaluateOutcome, why: &str) -> CaptureStatus {
    CaptureStatus::NotCaptured { reason: format!("{}: {why}", o.as_str()) }
}

pub fn capture_from_dir(dir: &Path) -> Vec<Capture> {
    EvaluateOutcome::ALL
        .iter()
        .map(|&o| {
            let path = dir.join(format!("{}.json", o.as_str()));
            let status = match std::fs::read_to_string(&path) {
                Err(_) => missing(o, "no fixture recorded"),
                Ok(text) => match serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()).and_then(|d| judge(o, &d)) {
                    Ok(source) => CaptureStatus::Captured { source },
                    Err(why) => missing(o, &why),
                },
            };
            Capture { outcome: o, status }
        })
        .collect()
}

pub fn capture_six() -> Vec<Capture> {
    capture_from_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR))
}

pub fn captured_count(c: &[Capture]) -> usize {
    c.iter().filter(|x| matches!(x.status, CaptureStatus::Captured { .. })).count()
}

/// (real, fault_injected) split of the captured outcomes; the headline "N of 6" must never hide a fault-injected one.
pub fn captured_split(c: &[Capture]) -> (usize, usize) {
    let f = c.iter().filter(|x| matches!(&x.status, CaptureStatus::Captured { source } if source.contains("(fault-injected,"))).count();
    (captured_count(c) - f, f)
}
