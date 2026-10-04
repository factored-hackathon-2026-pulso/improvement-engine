//! Hand-typed 1.3.0 DTOs over `serde_json::Value` (no dependency on `crates/core`). Requests encode to the exact
//! wire body of the Python bridge goldens (`bridge-contract/examples/flows`); responses decode the goldens and keep
//! the raw body in `raw` so nothing the bridge adds is lost. Unknown response fields are tolerated (the schemas of
//! `CoreVersion`, `AliasState` and the dry-run result are open); unknown REQUEST fields are impossible by construction.
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "decode: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

pub(crate) fn err<T>(what: &str, why: impl fmt::Display) -> Result<T, DecodeError> {
    Err(DecodeError(format!("{what}: {why}")))
}

pub(crate) fn obj<'a>(what: &str, v: &'a Value) -> Result<&'a Map<String, Value>, DecodeError> {
    v.as_object().ok_or_else(|| DecodeError(format!("{what}: not a JSON object")))
}

pub(crate) fn req_str(what: &str, m: &Map<String, Value>, k: &str) -> Result<String, DecodeError> {
    match m.get(k) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => err(what, format!("`{k}` is not a string")),
        None => err(what, format!("missing `{k}`")),
    }
}

/// A key that must be present but may be null.
pub(crate) fn nullable_str(what: &str, m: &Map<String, Value>, k: &str) -> Result<Option<String>, DecodeError> {
    match m.get(k) {
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Null) => Ok(None),
        Some(_) => err(what, format!("`{k}` is not a string or null")),
        None => err(what, format!("missing `{k}`")),
    }
}

pub(crate) fn opt_str(m: &Map<String, Value>, k: &str) -> Option<String> {
    m.get(k).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn opt_arr(m: &Map<String, Value>, k: &str) -> Vec<Value> {
    m.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

pub(crate) fn put(m: &mut Map<String, Value>, k: &str, v: Option<&str>) {
    if let Some(v) = v {
        m.insert(k.into(), Value::String(v.into()));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Run envelope (CoreTaskReceipt)
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Prepared,
    Sent,
    BindingConfirmed,
    TerminalOk,
    TerminalFailed,
    Unknown,
    ManualReconcile,
}

impl RunState {
    pub fn parse(s: &str) -> Option<RunState> {
        Some(match s {
            "prepared" => RunState::Prepared,
            "sent" => RunState::Sent,
            "binding_confirmed" => RunState::BindingConfirmed,
            "terminal_ok" => RunState::TerminalOk,
            "terminal_failed" => RunState::TerminalFailed,
            "unknown" => RunState::Unknown,
            "manual_reconcile" => RunState::ManualReconcile,
            _ => return None,
        })
    }
    pub fn is_terminal(self) -> bool {
        matches!(self, RunState::TerminalOk | RunState::TerminalFailed)
    }
}

/// `CoreTaskReceiptDetail`: stored with a terminal receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceiptDetail {
    pub core_run_id: String,
    pub release_id: String,
    pub outcome: String,
    pub trace_id: Option<String>,
    pub idempotency_key_digest: String,
    pub request_digest: String,
    pub task_binding_ref: String,
    pub input_commitment: String,
    pub output_refs: Vec<Value>,
    pub output_digest: Option<String>,
    pub audit_refs: Vec<Value>,
    pub budget_known: bool,
}

impl ReceiptDetail {
    fn decode(v: &Value) -> Result<ReceiptDetail, DecodeError> {
        const W: &str = "receipt";
        let m = obj(W, v)?;
        Ok(ReceiptDetail {
            core_run_id: req_str(W, m, "core_run_id")?,
            release_id: req_str(W, m, "release_id")?,
            outcome: req_str(W, m, "outcome")?,
            trace_id: nullable_str(W, m, "trace_id")?,
            idempotency_key_digest: req_str(W, m, "idempotency_key_digest")?,
            request_digest: req_str(W, m, "request_digest")?,
            task_binding_ref: req_str(W, m, "task_binding_ref")?,
            input_commitment: req_str(W, m, "input_commitment")?,
            output_refs: opt_arr(m, "output_refs"),
            output_digest: nullable_str(W, m, "output_digest")?,
            audit_refs: opt_arr(m, "audit_refs"),
            budget_known: m
                .get("budget")
                .and_then(|b| b.get("known"))
                .and_then(Value::as_bool)
                .ok_or_else(|| DecodeError("receipt: missing budget.known".into()))?,
        })
    }
}

/// One whitelisted fact of a run result.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    pub digest: String,
    pub source_kind: String,
    pub value: Value,
}

/// `ReadRunResult`: the projection of a run (whitelisted facts only).
#[derive(Debug, Clone, PartialEq)]
pub struct RunResult {
    pub core_run_id: String,
    pub status: String,
    pub outcome: String,
    pub output_digest: String,
    pub facts: BTreeMap<String, Fact>,
}

impl RunResult {
    fn decode(v: &Value) -> Result<RunResult, DecodeError> {
        const W: &str = "result";
        let m = obj(W, v)?;
        let mut facts = BTreeMap::new();
        for (k, f) in obj(W, m.get("facts").ok_or_else(|| DecodeError("result: missing facts".into()))?)? {
            let fm = obj("fact", f)?;
            facts.insert(
                k.clone(),
                Fact {
                    digest: req_str("fact", fm, "digest")?,
                    source_kind: req_str("fact", fm, "source_kind")?,
                    value: fm.get("value").cloned().ok_or_else(|| DecodeError(format!("fact {k}: missing value")))?,
                },
            );
        }
        Ok(RunResult {
            core_run_id: req_str(W, m, "core_run_id")?,
            status: req_str(W, m, "status")?,
            outcome: req_str(W, m, "outcome")?,
            output_digest: req_str(W, m, "output_digest")?,
            facts,
        })
    }
}

/// The run envelope: body of invoke 200/202 and of read 200. A non-terminal `state` (202) is a state, not a failure;
/// a `terminal_failed` run is still a decoded run, never an `Err`.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskReceipt {
    pub http_status: u16,
    pub state: RunState,
    pub core_run_id: Option<String>,
    pub outcome: Option<String>,
    pub reason: Option<String>,
    pub task_binding_ref: String,
    pub receipt: Option<ReceiptDetail>,
    pub result: Option<RunResult>,
    /// `pulso:task_unknown` / `pulso:task_in_progress`, only on a non-terminal read.
    pub code: Option<String>,
    pub trace_id: Option<String>,
    pub proven_no_effect: Option<bool>,
    pub adopted_writes: Vec<Value>,
    pub raw: Value,
}

impl TaskReceipt {
    pub fn from_response(http_status: u16, body: &Value) -> Result<TaskReceipt, DecodeError> {
        const W: &str = "CoreTaskReceipt";
        let m = obj(W, body)?;
        if m.get("schema_version").and_then(Value::as_str) != Some("1") {
            return err(W, "schema_version is not \"1\"");
        }
        let state_s = req_str(W, m, "state")?;
        let state = RunState::parse(&state_s).ok_or_else(|| DecodeError(format!("{W}: unknown state {state_s:?}")))?;
        Ok(TaskReceipt {
            http_status,
            state,
            core_run_id: nullable_str(W, m, "core_run_id")?,
            outcome: nullable_str(W, m, "outcome")?,
            reason: nullable_str(W, m, "reason")?,
            task_binding_ref: req_str(W, m, "task_binding_ref")?,
            receipt: m.get("receipt").map(ReceiptDetail::decode).transpose()?,
            result: m.get("result").map(RunResult::decode).transpose()?,
            code: opt_str(m, "code"),
            trace_id: opt_str(m, "trace_id"),
            proven_no_effect: m.get("proven_no_effect").and_then(Value::as_bool),
            adopted_writes: opt_arr(m, "adopted_writes"),
            raw: body.clone(),
        })
    }

    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// Terminal and the run completed (`terminal_ok`, outcome `completed`).
    pub fn is_success(&self) -> bool {
        self.state == RunState::TerminalOk && self.outcome.as_deref() == Some("completed")
    }

    pub fn fact(&self, name: &str) -> Option<&Fact> {
        self.result.as_ref().and_then(|r| r.facts.get(name))
    }
}
