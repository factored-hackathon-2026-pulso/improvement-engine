//! V2: gate wiring over arm reports through the GSI stand-in (`steps::gate`, `gate=claude-authored`), reused through
//! the steps API, not copied. Author is not judge: a judge that coincides with any author (builder, world, suite)
//! makes both gates `not_evaluable`. Structural verdict only; `quality_claims` stays forbidden.
use core_client::dto::ArmReport;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    NotEvaluable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateStatus {
    Pass,
    Fail,
    NotEvaluable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    pub gate: String,
    pub status: GateStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateVerdict {
    pub verdict: Verdict,
    pub gates: Vec<GateResult>,
    pub judge_actor: String,
    /// Authorship label of the gate.
    pub label: String,
    /// Semantics label of the verdict judge.
    pub semantics: String,
    pub quality_claims: String,
    pub suite_ref: String,
}

pub struct GateInput<'a> {
    /// Digest of the sealed suite (`EvalPackage::suite_digest`, 64 lowercase hex); `suite_ref` is derived from it.
    pub suite_digest: &'a str,
    pub run_id: &'a str,
    pub judge_actor: &'a str,
    pub author_actors: Vec<&'a str>,
    pub world_author: &'a str,
    pub suite_author: &'a str,
    pub base: &'a [ArmReport],
    pub candidate: &'a [ArmReport],
}

fn runs(rs: &[ArmReport]) -> Value {
    let v: Vec<Value> = rs
        .iter()
        .map(|r| {
            json!({
                "case_ref": r.case_ref, "status": r.raw.get("status"), "closed_early": r.closed_early,
                "cost_known": r.cost_known, "oracle_ref": r.oracle_ref,
            })
        })
        .collect();
    json!({ "runs": v })
}

fn status(s: &str) -> Result<GateStatus, String> {
    match s {
        "pass" => Ok(GateStatus::Pass),
        "fail" => Ok(GateStatus::Fail),
        "not_evaluable" => Ok(GateStatus::NotEvaluable),
        o => Err(format!("unknown gate status {o:?}")),
    }
}

/// The exact input document of the gate step (`steps::gate::run`): `gate_in`, the arm `reports` and `world_authors`.
/// Exposed so a job handler can put the live arm reports into the payload of the gate step.
pub fn gate_env(i: &GateInput) -> Result<Value, String> {
    if i.suite_digest.len() != 64 || !i.suite_digest.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
        return Err("suite_digest must be the 64-hex sealed suite digest".into());
    }
    let suite_ref = format!("eval_suite:{}@1", i.suite_digest);
    let (b, c) = (format!("arm_report:base-{}@1", i.run_id), format!("arm_report:cand-{}@1", i.run_id));
    Ok(json!({
        "gate_in": {
            "contract_version": "engine-steps/0", "step": "gate", "run_id": i.run_id, "data_class": "synthetic",
            "base_arm_report_ref": b, "candidate_arm_report_ref": c, "suite_ref": &suite_ref,
            "judge_actor": i.judge_actor, "author_actors": i.author_actors,
        },
        "reports": { b: runs(i.base), c: runs(i.candidate) },
        "world_authors": { "world": i.world_author, "suite": i.suite_author },
    }))
}

pub fn wire_gate(i: &GateInput) -> Result<GateVerdict, String> {
    let env = gate_env(i)?;
    let suite_ref = format!("eval_suite:{}@1", i.suite_digest);
    let out = steps::gate::run(&env.to_string()).map_err(|e| e.to_string())?;
    let o: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    let verdict = match o["verdict"].as_str() {
        Some("pass") => Verdict::Pass,
        Some("fail") => Verdict::Fail,
        Some("not_evaluable") => Verdict::NotEvaluable,
        v => return Err(format!("unknown verdict {v:?}")),
    };
    let mut gates = Vec::new();
    for g in o["gates"].as_array().ok_or("gates missing")? {
        gates.push(GateResult {
            gate: g["gate"].as_str().ok_or("gate name")?.into(),
            status: status(g["status"].as_str().ok_or("gate status")?)?,
            reason: g["reason"].as_str().map(str::to_string),
        });
    }
    if gates.len() != 2 {
        return Err("both gates are required".into());
    }
    Ok(GateVerdict {
        verdict,
        gates,
        judge_actor: i.judge_actor.into(),
        label: steps::gate::LABEL.into(),
        semantics: steps::SEMANTICS.into(),
        quality_claims: match o["quality_claims"].as_str() {
            Some("forbidden") => "forbidden".into(),
            other => return Err(format!("quality_claims must be forbidden, got {other:?}")),
        },
        suite_ref,
    })
}
