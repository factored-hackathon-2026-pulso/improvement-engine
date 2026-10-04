//! Fixture-based golden capture of the six evaluate outcomes. Only recorded real reports count; an outcome with no
//! recorded real report is `NotCaptured` with the reason (never fabricated).
use crate::outcomes::{ArmRollup, EvaluateOutcome, classify_http, rollup_arms};
use core_client::dto::ArmReport;
use serde_json::Value;

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

fn step_body(flow: &str, case: &str) -> Option<Value> {
    let d: Value = serde_json::from_str(flow).ok()?;
    d["steps"].as_array()?.iter().find(|s| s["case"] == case).map(|s| s["response"]["body"].clone())
}

fn pass() -> Option<String> {
    let b = step_body(WRITER_EVALUATION, "invoke_evaluate_only")?;
    let ne = &b["result"]["facts"]["pulso_writer_receipts"]["value"]["native_evaluation"];
    (classify_http(200, ne) == Ok(EvaluateOutcome::Pass)).then(|| "bridge-contract/examples/flows/writer_evaluation.json#invoke_evaluate_only (native_evaluation.verdict=pass)".into())
}

fn failed_infra() -> Option<String> {
    let b = step_body(ARMS, "arm_budget_unknown")?;
    let r = ArmReport::from_json(&b).ok()?;
    (rollup_arms(&[r]) == ArmRollup::Outcome(EvaluateOutcome::FailedInfra)).then(|| "bridge-contract/examples/flows/arms.json#arm_budget_unknown (arm report status=failed_infra, arm-level)".into())
}

fn missing(o: EvaluateOutcome) -> &'static str {
    match o {
        EvaluateOutcome::Fail => "no recorded real arm report: the 409 gate_failed body is in no golden or e2e-core fixture",
        EvaluateOutcome::QuotaExceeded => "no recorded real 429 quota_exceeded evaluate response",
        EvaluateOutcome::CandidateChanged => "no recorded evaluate-level candidate_changed (only admission/approve-level exists)",
        EvaluateOutcome::ResultLost => "client-side timeout, no recorded real report by definition",
        _ => "not captured",
    }
}

pub fn capture_six() -> Vec<Capture> {
    EvaluateOutcome::ALL
        .iter()
        .map(|&o| {
            let hit = match o {
                EvaluateOutcome::Pass => pass(),
                EvaluateOutcome::FailedInfra => failed_infra(),
                _ => None,
            };
            let status = match hit {
                Some(source) => CaptureStatus::Captured { source },
                None => CaptureStatus::NotCaptured { reason: missing(o).into() },
            };
            Capture { outcome: o, status }
        })
        .collect()
}

pub fn captured_count(c: &[Capture]) -> usize {
    c.iter().filter(|x| matches!(x.status, CaptureStatus::Captured { .. })).count()
}
