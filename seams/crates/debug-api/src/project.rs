//! Projection: folds run events into the per-run state the read routes serve. Pure and total: an unknown event kind
//! is stored in the log but changes nothing here (forward compatible), a malformed payload is ignored field by field.
//!
//! State shape: `{run, nodes, investigation, gates, alternatives, diff, decision, doubles, memory}`.
use serde_json::{Value, json};

pub fn empty_state(run_id: &str) -> Value {
    json!({
        "run": {"run_id": run_id, "title": run_id, "state": "queued", "origin": "manual"},
        "nodes": [], "investigation": null, "gates": null, "alternatives": [], "diff": null, "decision": null,
        "doubles": [], "memory": [],
    })
}

fn set_str(dst: &mut Value, key: &str, src: &Value) {
    if let Some(s) = src.get(key).and_then(Value::as_str) {
        dst[key] = Value::String(s.to_string());
    }
}

pub fn apply(state: &mut Value, event: &Value) {
    let data = &event["data"];
    match event["kind"].as_str().unwrap_or_default() {
        "run_started" => {
            for k in ["title", "state", "origin"] {
                set_str(&mut state["run"], k, data);
            }
        }
        "run_state_changed" => set_str(&mut state["run"], "state", data),
        "node_status_changed" => {
            let (Some(node), Some(list)) = (data.get("node").filter(|n| n["node_id"].is_string()), state["nodes"].as_array_mut()) else { return };
            match list.iter_mut().find(|n| n["node_id"] == node["node_id"]) {
                Some(slot) => *slot = node.clone(),
                None => list.push(node.clone()),
            }
        }
        "investigation_set" => state["investigation"] = data.clone(),
        "gates_set" => state["gates"] = data.clone(),
        "alternatives_set" => {
            if let Some(items) = data.get("items").filter(|i| i.is_array()) {
                state["alternatives"] = items.clone();
            }
        }
        "diff_set" => {
            if data["proposal_id"].is_string() && data["lines"].is_array() {
                state["diff"] = data.clone();
            }
        }
        "decision_set" => {
            if data["decision_id"].is_string() {
                state["decision"] = data.clone();
            }
        }
        "doubles_declared" => {
            let (Some(new), Some(list)) = (data.get("doubles").and_then(Value::as_array), state["doubles"].as_array_mut()) else { return };
            for d in new {
                if d["id"].is_string() && !list.iter().any(|x| x["id"] == d["id"]) {
                    list.push(d.clone());
                }
            }
        }
        "memory_set" => {
            if let Some(items) = data.get("items").filter(|i| i.is_array()) {
                state["memory"] = items.clone();
            }
        }
        _ => {}
    }
}
