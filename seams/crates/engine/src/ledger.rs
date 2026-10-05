//! Viability ledger: every proposal ends in a recorded verdict (`viable` | `not_viable` | `not_evaluable`) with a named
//! reason from a closed vocabulary, the hypotheses and evidence refs it came from, the gate results and any override
//! (always labelled simulated). Entries are immutable once stored: `ledger/e<ordinal>` in the engine job store, created
//! with compare-and-set, so a replay of the same entry is a no-op and a different entry under the same ordinal is refused.
//!
//! The one rule that matters: `viable` only when the structural gate said `pass` on its own (no override anywhere on the
//! trail). `validate` enforces it, `Ledger::record` refuses to store an entry that fails it.
use crate::JobStore;
use serde_json::{Value, json};

const BK0: &str = include_str!("../../../../contracts/artifact-kinds/matrix.json");

pub const VIABLE_REASONS: [&str; 1] = ["structural_gate_passed"];
pub const NOT_VIABLE_REASONS: [&str; 4] = ["gate_failed", "native_eval_failed", "verifier_refuted", "claim_not_corroborated"];
pub const NOT_EVALUABLE_REASONS: [&str; 10] = [
    "kind_not_supported",
    "release_settings_not_allowed",
    "gate_not_evaluable",
    "evidence_insufficient",
    "model_refused",
    "model_unavailable",
    "model_invalid",
    "precondition_missing",
    "compile_denied",
    "core_unavailable",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Viable,
    NotViable,
    NotEvaluable,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Viable => "viable",
            Verdict::NotViable => "not_viable",
            Verdict::NotEvaluable => "not_evaluable",
        }
    }
    pub fn parse(s: &str) -> Option<Verdict> {
        [Verdict::Viable, Verdict::NotViable, Verdict::NotEvaluable].into_iter().find(|v| v.as_str() == s)
    }
    pub fn reasons(self) -> &'static [&'static str] {
        match self {
            Verdict::Viable => &VIABLE_REASONS,
            Verdict::NotViable => &NOT_VIABLE_REASONS,
            Verdict::NotEvaluable => &NOT_EVALUABLE_REASONS,
        }
    }
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
    /// The last stage the proposal reached (`scout`, `verifier`, `builder`, `compile`, `gate`, ...).
    pub stage: String,
    pub kind: Option<String>,
    pub op: Option<String>,
    pub hypotheses: Vec<Value>,
    pub evidence_refs: Vec<String>,
    pub gates: Vec<GateResult>,
    pub gate_verdict: Option<String>,
    /// Overrides on the trail, each labelled and `simulated: true`.
    pub overrides: Vec<Value>,
    /// One record per model call (`CallRecord::to_json`).
    pub models: Vec<Value>,
    pub label: String,
}

/// BK0 (`contracts/artifact-kinds/matrix.json`, embedded at build time): is `(kind, op)` a supported change family?
/// `Err(reason)` carries the matrix reason (`kind_not_supported`, `release_settings_not_allowed`).
pub fn bk0_check(kind: &str, op: &str) -> Result<(), String> {
    let m: Value = serde_json::from_str(BK0).map_err(|e| format!("BK0 matrix is not JSON: {e}"))?;
    if let Some(f) = m["change_families"].as_array().and_then(|a| a.iter().find(|f| f["target_kind"] == kind && f["op"] == op)) {
        return match f["verdict"].as_str() {
            Some("supported") => Ok(()),
            _ => Err(f["reason"].as_str().unwrap_or("kind_not_supported").to_string()),
        };
    }
    let denied = m["denied_kinds"].as_array().and_then(|a| a.iter().find(|d| d["kind"] == kind));
    Err(denied.and_then(|d| d["reason"].as_str()).unwrap_or("kind_not_supported").to_string())
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

    /// The invariants of a recorded verdict (see the module doc). `Ok` does not mean the proposal is good, only that the
    /// entry cannot lie about how it got its verdict.
    pub fn validate(&self) -> Result<(), String> {
        if !self.verdict.reasons().contains(&self.reason.as_str()) {
            return Err(format!("reason {:?} does not belong to verdict {} (allowed: {:?})", self.reason, self.verdict.as_str(), self.verdict.reasons()));
        }
        for o in &self.overrides {
            if o.get("label").and_then(Value::as_str).is_none_or(str::is_empty) {
                return Err("an override on the trail has no label".into());
            }
            if o.get("simulated") != Some(&Value::Bool(true)) {
                return Err("an override on the trail is not labelled simulated".into());
            }
        }
        if self.verdict == Verdict::Viable {
            if self.gate_verdict.as_deref() != Some("pass") {
                return Err(format!("viable needs gate verdict pass, got {:?}", self.gate_verdict));
            }
            match (self.kind.as_deref(), self.op.as_deref()) {
                (Some(k), Some(o)) => bk0_check(k, o).map_err(|why| format!("viable needs a BK0-supported change family; ({k},{o}): {why}"))?,
                _ => return Err("viable needs a BK0-supported change family: kind and op are missing".into()),
            }
            if self.gates.is_empty() || self.gates.iter().any(|g| g.status != "pass") {
                return Err("viable needs every recorded gate to be pass".into());
            }
            if !self.overrides.is_empty() {
                return Err("viable cannot have an override on its trail: an overridden gate is not a pass".into());
            }
            if self.evidence_refs.is_empty() {
                return Err("viable needs evidence refs".into());
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        json!({"ordinal": self.ordinal, "run_id": self.run_id, "proposal_id": self.proposal_id, "signal_id": self.signal_id,
               "verdict": self.verdict.as_str(), "reason": self.reason, "detail": self.detail, "stage": self.stage,
               "kind": self.kind, "op": self.op, "hypotheses": self.hypotheses, "evidence_refs": self.evidence_refs,
               "gates": self.gates.iter().map(|g| json!({"gate": g.gate, "status": g.status})).collect::<Vec<_>>(),
               "gate_verdict": self.gate_verdict, "overrides": self.overrides, "models": self.models, "label": self.label,
               "quality_claims": "forbidden"})
    }

    pub fn from_json(v: &Value) -> Result<LedgerEntry, String> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("ledger entry.{k} missing"));
        let opt = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let list = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        let verdict = Verdict::parse(&s("verdict")?).ok_or("ledger entry.verdict unknown")?;
        let mut gates = vec![];
        for g in list("gates") {
            let f = |k: &str| g.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("ledger entry gate.{k} missing"));
            gates.push(GateResult { gate: f("gate")?, status: f("status")? });
        }
        Ok(LedgerEntry {
            ordinal: v.get("ordinal").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).ok_or("ledger entry.ordinal missing")?,
            run_id: s("run_id")?,
            proposal_id: s("proposal_id")?,
            signal_id: s("signal_id")?,
            verdict,
            reason: s("reason")?,
            detail: s("detail")?,
            stage: s("stage")?,
            kind: opt("kind"),
            op: opt("op"),
            hypotheses: list("hypotheses"),
            evidence_refs: list("evidence_refs").iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            gates,
            gate_verdict: opt("gate_verdict"),
            overrides: list("overrides"),
            models: list("models"),
            label: s("label")?,
        })
    }
}

pub struct Ledger<'a> {
    store: &'a dyn JobStore,
}

impl<'a> Ledger<'a> {
    pub fn new(store: &'a dyn JobStore) -> Ledger<'a> {
        Ledger { store }
    }

    fn key(ordinal: u32) -> String {
        format!("ledger/e{ordinal:04}")
    }

    /// Store `e` (validated). `Ok(true)` = stored now, `Ok(false)` = the identical entry was already there.
    pub fn record(&self, e: &LedgerEntry) -> Result<bool, String> {
        e.validate()?;
        let (key, text) = (Self::key(e.ordinal), e.to_json().to_string());
        match self.store.cas(&key, 0, &text) {
            Ok(_) => Ok(true),
            Err(conflict) => match self.store.get(&key)? {
                Some((_, have)) if have == text => Ok(false),
                Some(_) => Err(format!("ledger entry {} is immutable: a different entry is already recorded", e.ordinal)),
                None => Err(conflict),
            },
        }
    }

    /// All recorded entries in ordinal order (ordinals are dense from 0).
    pub fn entries(&self) -> Result<Vec<LedgerEntry>, String> {
        let mut out = vec![];
        for i in 0u32.. {
            match self.store.get(&Self::key(i))? {
                Some((_, text)) => out.push(LedgerEntry::from_json(&serde_json::from_str(&text).map_err(|e| format!("corrupt ledger entry {i}: {e}"))?)?),
                None => break,
            }
        }
        Ok(out)
    }
}
