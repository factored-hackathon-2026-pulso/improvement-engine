//! The write side: what an engine hands to a sink. The store assigns the sequence and the wire fields.
use serde_json::Value;

/// One thing that happened in a run. `kind` is the event code (`run_started`, `node_status_changed`, ...);
/// `data` carries the payload the projection folds (see `project`).
#[derive(Clone, Debug)]
pub struct NewEvent {
    pub kind: String,
    pub entity_kind: String,
    pub entity_id: String,
    pub data: Value,
    /// RFC 3339 UTC; `None` = now.
    pub occurred_at: Option<String>,
}

impl NewEvent {
    pub fn new(kind: &str, entity_kind: &str, entity_id: &str, data: Value) -> NewEvent {
        NewEvent { kind: kind.into(), entity_kind: entity_kind.into(), entity_id: entity_id.into(), data, occurred_at: None }
    }
}

/// What a running engine calls to stream events. Returns the stored wire event (with its sequence).
pub trait RunEventSink: Send + Sync {
    fn emit(&self, run_id: &str, ev: NewEvent) -> Result<Value, String>;
}
