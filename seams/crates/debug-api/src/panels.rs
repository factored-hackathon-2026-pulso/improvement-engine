//! Panel projection (stub: RED). See the GREEN commit.
use crate::event::NewEvent;
use serde_json::Value;

pub fn project(_committed: &Value, _report: Option<&Value>, _at: &str) -> Vec<NewEvent> {
    vec![]
}
