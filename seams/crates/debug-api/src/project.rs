//! Projection: folds run events into the per-run state the read routes serve.
use serde_json::Value;

pub fn empty_state(run_id: &str) -> Value {
    let _ = run_id;
    todo!()
}

pub fn apply(state: &mut Value, event: &Value) {
    let _ = (state, event);
    todo!()
}
