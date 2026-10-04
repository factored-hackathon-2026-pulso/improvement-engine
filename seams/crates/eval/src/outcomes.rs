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

/// Classify a recorded observation of one evaluate attempt (the shape stored in `tests/fixtures/v1`):
/// - `stage_receipt`: the evaluate-only writer stage completed. A native verdict decides. NO verdict plus a verified
///   `evaluate` write and the proposal back in `draft` is the failed gate (409 `gate_failed` inside Core's registry tool:
///   the bridge does not pass it through as HTTP on this profile; the report stays in Core's `eval_reports`). No verdict
///   with the proposal still frozen is NOT a failed gate.
/// - `http_error`: the bridge's refusal (`classify_http`).
/// - `timeout`: only a request that was SENT can be a lost result.
pub fn classify_observation(obs: &Value) -> Result<EvaluateOutcome, String> {
    match obs.get("kind").and_then(Value::as_str) {
        Some("stage_receipt") => {
            let native = obs.get("native_evaluation").filter(|n| !n.is_null());
            if let Some(n) = native {
                return classify_http(200, n);
            }
            let wrote_eval = obs.get("write_ops").and_then(Value::as_array).is_some_and(|o| o.len() == 1 && o[0] == "evaluate");
            match (wrote_eval, obs.get("proposal_state_after").and_then(Value::as_str)) {
                (true, Some("draft")) => Ok(EvaluateOutcome::Fail),
                (_, state) => Err(format!("a completed stage without a verdict is a failed gate only when the proposal is back in draft (proposal state {state:?}, evaluate write {wrote_eval})")),
            }
        }
        Some("http_error") => {
            let status = obs.get("http_status").and_then(Value::as_u64).ok_or("http_error without http_status")? as u16;
            classify_http(status, &serde_json::json!({"code": obs.get("code").and_then(Value::as_str).unwrap_or("")}))
        }
        Some("timeout") => {
            if obs.get("request_sent").and_then(Value::as_bool) == Some(true) { Ok(classify_timeout()) } else { Err("a timeout of a request that was never sent is not a lost result".into()) }
        }
        other => Err(format!("unknown observation kind {other:?}")),
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
