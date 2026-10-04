//! The six `evaluate` outcomes of the real Core (spec V3 31.7.1, CAP-38) and their classification.
use core_client::dto::{ArmReport, ArmStatus};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluateOutcome {
    Pass,
    /// 409 `gate_failed` with the EvalReport as payload.
    Fail,
    FailedInfra,
    /// 429 `quota_exceeded`.
    QuotaExceeded,
    /// 409 `candidate_changed`.
    CandidateChanged,
    /// Client timeout: `pulso:evaluation_result_lost` until the proposal is re-read.
    ResultLost,
}

impl EvaluateOutcome {
    pub const ALL: [EvaluateOutcome; 6] = [Self::Pass, Self::Fail, Self::FailedInfra, Self::QuotaExceeded, Self::CandidateChanged, Self::ResultLost];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::FailedInfra => "failed_infra",
            Self::QuotaExceeded => "quota_exceeded",
            Self::CandidateChanged => "candidate_changed",
            Self::ResultLost => "evaluation_result_lost",
        }
    }
}

fn code(body: &Value) -> &str {
    let c = body.get("code").and_then(Value::as_str).unwrap_or("");
    c.strip_prefix("pulso:").unwrap_or(c)
}

/// Classify one evaluate response (`body` is the EvalReport on 200, the error envelope otherwise).
pub fn classify_http(status: u16, body: &Value) -> Result<EvaluateOutcome, String> {
    match (status, code(body)) {
        (200, _) => match body.get("verdict").and_then(Value::as_str) {
            Some("pass") => Ok(EvaluateOutcome::Pass),
            Some("failed_infra") => Ok(EvaluateOutcome::FailedInfra),
            Some("fail") => Ok(EvaluateOutcome::Fail),
            v => Err(format!("unknown verdict {v:?}")),
        },
        (409, "gate_failed") => Ok(EvaluateOutcome::Fail),
        (409, "candidate_changed") => Ok(EvaluateOutcome::CandidateChanged),
        (429, "quota_exceeded") => Ok(EvaluateOutcome::QuotaExceeded),
        (s, c) => Err(format!("unmapped evaluate response {s} {c:?}")),
    }
}

pub fn classify_timeout() -> EvaluateOutcome {
    EvaluateOutcome::ResultLost
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmRollup {
    /// Every arm is `completed` (or there are none): the arms only say "no raise", never pass.
    NoVerdict,
    Outcome(EvaluateOutcome),
}

/// What arm reports alone can say. Core arms report only status/closed_early/cost/usage, so `completed` is not a
/// pass: `Pass` comes only from the native verdict.
pub fn rollup_arms(reports: &[ArmReport]) -> ArmRollup {
    let any = |s: ArmStatus| reports.iter().any(|r| r.status == s);
    if any(ArmStatus::FailedInfra) {
        ArmRollup::Outcome(EvaluateOutcome::FailedInfra)
    } else if any(ArmStatus::Unknown) {
        ArmRollup::Outcome(EvaluateOutcome::ResultLost)
    } else if any(ArmStatus::CandidateFailed) {
        ArmRollup::Outcome(EvaluateOutcome::Fail)
    } else {
        ArmRollup::NoVerdict
    }
}
