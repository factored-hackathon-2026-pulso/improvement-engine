//! Hand-typed 1.3.0 DTOs over `serde_json::Value` (no dependency on `crates/core`). STUB: RED phase.
use serde_json::Value;

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

/// The run envelope (`CoreTaskReceipt`): body of invoke 200/202 and of read 200.
#[derive(Debug, Clone)]
pub struct TaskReceipt {
    pub http_status: u16,
    pub state: RunState,
    pub core_run_id: Option<String>,
    pub task_binding_ref: String,
}

impl TaskReceipt {
    pub fn from_response(_http_status: u16, _body: &Value) -> Result<TaskReceipt, String> {
        Err("not implemented".into())
    }
}
