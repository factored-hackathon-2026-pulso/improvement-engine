//! V3r: rule-driven bounded revision. When the gate returns `fail` or `not_evaluable` (no verdict), the engine asks a
//! deterministic [`RevisionPolicy`] for a new [`ChangeSpec`] variant, at most [`MAX_REVISIONS`] times, then stops.
//! HONEST LABEL: [`ShrinkPolicy`] is a rule-driven stand-in (no model): it narrows the change limits and walks the
//! catalogue variant ladder; it does not judge quality. The spend ceiling is fixed at the start and never grows.
use crate::gate::{GateResult, GateVerdict, Verdict};
use core_client::canon::sha256_hex;

pub const MAX_REVISIONS: u32 = 2;
pub const POLICY_LABEL: &str = "revision=rule-driven stand-in (no model)";

/// Minimal change description: a catalogue variant plus the BK0-matrix limits it may touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSpec {
    pub variant: String,
    pub max_files: u32,
    pub max_lines: u32,
}

impl ChangeSpec {
    pub fn digest(&self) -> String {
        sha256_hex(format!("{}|{}|{}", self.variant, self.max_files, self.max_lines).as_bytes())
    }
}

pub trait RevisionPolicy {
    /// New spec from the previous one and the gate findings; `None` = nothing left to try.
    fn revise(&self, prev: &ChangeSpec, findings: &[GateResult], revision: u32) -> Option<ChangeSpec>;
}

/// Stand-in: halve the limits (floor 1) and tag the variant `+rN`. Deterministic.
pub struct ShrinkPolicy;

impl RevisionPolicy for ShrinkPolicy {
    fn revise(&self, prev: &ChangeSpec, _findings: &[GateResult], revision: u32) -> Option<ChangeSpec> {
        let base = prev.variant.split("+r").next().unwrap_or(&prev.variant);
        Some(ChangeSpec { variant: format!("{base}+r{revision}"), max_files: (prev.max_files / 2).max(1), max_lines: (prev.max_lines / 2).max(1) })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub n: u32,
    pub label: String,
    pub spec: ChangeSpec,
    /// Distinct per attempt: `sha256(run|label|spec digest)`.
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRecord {
    pub label: String,
    pub idempotency_key: String,
    pub spec: ChangeSpec,
    pub verdict: Verdict,
    pub spend: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Passed,
    RevisionsExhausted,
    PolicyExhausted,
    BudgetExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionOutcome {
    pub attempts: Vec<AttemptRecord>,
    pub stop: Stop,
    pub budget_ceiling: u64,
    pub spent: u64,
    pub policy_label: &'static str,
}

fn attempt(run_id: &str, n: u32, spec: ChangeSpec) -> Attempt {
    let label = if n == 0 { "initial".to_string() } else { format!("revision-{n}") };
    let idempotency_key = sha256_hex(format!("{run_id}|{label}|{}", spec.digest()).as_bytes());
    Attempt { n, label, spec, idempotency_key }
}

/// Run the initial attempt and at most [`MAX_REVISIONS`] revisions. `evaluate` returns the gate verdict and the spend of
/// that attempt. `Pass` stops; `Fail`/`NotEvaluable` revise; the ceiling is checked before every attempt.
pub fn run_bounded<F>(run_id: &str, initial: ChangeSpec, ceiling: u64, policy: &dyn RevisionPolicy, mut evaluate: F) -> Result<RevisionOutcome, String>
where
    F: FnMut(&Attempt) -> Result<(GateVerdict, u64), String>,
{
    let mut attempts = Vec::new();
    let mut spent = 0u64;
    let mut spec = initial;
    let mut n = 0u32;
    let stop = loop {
        if spent >= ceiling {
            break Stop::BudgetExhausted;
        }
        let a = attempt(run_id, n, spec.clone());
        let (v, spend) = evaluate(&a)?;
        spent = spent.saturating_add(spend);
        attempts.push(AttemptRecord { label: a.label, idempotency_key: a.idempotency_key, spec: spec.clone(), verdict: v.verdict, spend });
        if v.verdict == Verdict::Pass {
            break Stop::Passed;
        }
        if n >= MAX_REVISIONS {
            break Stop::RevisionsExhausted;
        }
        n += 1;
        match policy.revise(&spec, &v.gates, n) {
            Some(s) => spec = s,
            None => break Stop::PolicyExhausted,
        }
    };
    Ok(RevisionOutcome { attempts, stop, budget_ceiling: ceiling, spent, policy_label: POLICY_LABEL })
}
