//! Viability ledger: every proposal ends in a recorded verdict.
use crate::JobStore;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Viable,
    NotViable,
    NotEvaluable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    pub gate: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LedgerEntry {
    pub ordinal: u32,
    pub run_id: String,
    pub proposal_id: String,
    pub signal_id: String,
    pub verdict: Verdict,
    pub reason: String,
    pub detail: String,
    pub stage: String,
    pub kind: Option<String>,
    pub op: Option<String>,
    pub hypotheses: Vec<Value>,
    pub evidence_refs: Vec<String>,
    pub gates: Vec<GateResult>,
    pub gate_verdict: Option<String>,
    pub overrides: Vec<Value>,
    pub models: Vec<Value>,
    pub label: String,
}

pub fn bk0_check(_kind: &str, _op: &str) -> Result<(), String> {
    todo!("RED")
}

impl LedgerEntry {
    pub fn new(ordinal: u32, run_id: &str, proposal_id: &str, signal_id: &str, verdict: Verdict, reason: &str) -> LedgerEntry {
        LedgerEntry {
            ordinal,
            run_id: run_id.into(),
            proposal_id: proposal_id.into(),
            signal_id: signal_id.into(),
            verdict,
            reason: reason.into(),
            detail: String::new(),
            stage: String::new(),
            kind: None,
            op: None,
            hypotheses: vec![],
            evidence_refs: vec![],
            gates: vec![],
            gate_verdict: None,
            overrides: vec![],
            models: vec![],
            label: "DEMO-0".into(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        todo!("RED")
    }
    pub fn to_json(&self) -> Value {
        todo!("RED")
    }
    pub fn from_json(_v: &Value) -> Result<LedgerEntry, String> {
        todo!("RED")
    }
}

pub struct Ledger<'a> {
    store: &'a dyn JobStore,
}

impl<'a> Ledger<'a> {
    pub fn new(store: &'a dyn JobStore) -> Ledger<'a> {
        Ledger { store }
    }
    pub fn record(&self, _e: &LedgerEntry) -> Result<bool, String> {
        todo!("RED")
    }
    pub fn entries(&self) -> Result<Vec<LedgerEntry>, String> {
        todo!("RED")
    }
}
