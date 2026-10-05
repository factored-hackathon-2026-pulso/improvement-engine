//! Engine-run report (contracts/engine-run, C-2) -> run events, plus the contract seed.
//! The report carries labels and ids, never rows; every `doubles[]` entry becomes a visible double and every step keeps its
//! honesty label in the node label, so nothing a report admits to being a stand-in is hidden.
use crate::event::{NewEvent, RunEventSink};
use serde_json::{Value, json};

fn stage_of(step: &str) -> &str {
    match step {
        "trigger" => "trigger",
        "signals" | "scout" => "scout",
        "opportunity" => "hypothesis",
        "verifier" => "verifier",
        "compile" | "revision" => "proposal",
        "gate" => "evaluation",
        "approval" => "decision",
        other => other,
    }
}

/// One `doubles_declared` item for a `doubles[]` entry of the report.
pub fn double_item(d: &Value) -> Option<Value> {
    let (part, status) = (d["part"].as_str().filter(|x| !x.is_empty())?, d["status"].as_str().filter(|x| !x.is_empty())?);
    Some(json!({"id": format!("{part}:{status}"), "what": double_what(d)}))
}

/// The graph node of a report step. `status` is the report's honesty label, or `pending` / `running` while a live run has not
/// finished it; the label always carries it (`<id> [<status>]`).
pub fn step_node(id: &str, status: &str, prev: Option<&str>) -> Value {
    let (node_status, reason) = if status == "pending" {
        ("planned", None)
    } else if status == "running" {
        ("running", None)
    } else if status.starts_with("not_exercised") {
        ("planned", Some("not_exercised"))
    } else if status.starts_with("blocked") {
        ("waiting_dependency", Some("dependency_blocked"))
    } else {
        ("complete", None)
    };
    json!({
        "node_id": id, "label": format!("{id} [{status}]"), "stage": stage_of(id), "status": node_status,
        "depends_on": prev.map(|p| vec![p]).unwrap_or_default(), "reason_code": reason, "node_kind": "material_step", "trace_id": null,
    })
}

/// The `gates_set` payload of a report (the native verdict; the improvement gate is not in the report).
pub fn gates_data(report: &Value) -> Value {
    let verdict = report["gate"]["verdict"].as_str().unwrap_or("unknown");
    json!({
        "native": {"status": verdict, "reason_code": null, "report_ref": null, "checked_at": null},
        "improvement": {"status": "not_evaluable", "reason_code": "not_in_engine_run_report", "receipt_refs": [], "checked_at": null},
        "combined": {"decision": "hold", "reason_code": "improvement_gate_not_in_report"},
        "proposal_id": null, "attempts": [],
    })
}

fn double_what(d: &Value) -> String {
    let mut s = d["status"].as_str().unwrap_or("unknown").to_string();
    if d["observed"] == true {
        s.push_str(" (observed fact)");
    }
    for (k, label) in [("provider", "provider"), ("data_class", "data_class"), ("price_source", "price_source"), ("verdict", "verdict"), ("by", "by")] {
        if let Some(v) = d[k].as_str() {
            s.push_str(&format!(", {label} {v}"));
        }
    }
    s
}

/// Turns an engine-run report into a finished run in `sink`; returns the run id. Refuses a report that claims quality,
/// that has no steps, or that was already ingested (`exists`).
pub fn engine_run_report(sink: &dyn RunEventSink, exists: &dyn Fn(&str) -> bool, report: &Value) -> Result<String, String> {
    let label = report["label"].as_str().filter(|l| !l.is_empty()).ok_or("report.label is required")?;
    let sha = report["sha"].as_str().filter(|l| !l.is_empty()).ok_or("report.sha is required")?;
    if report["quality_claims"] != "forbidden" {
        return Err("report.quality_claims must be \"forbidden\"".into());
    }
    let steps = report["steps"].as_array().filter(|s| !s.is_empty()).ok_or("report.steps must be a non-empty array")?;
    let run = format!("run-{}-{}", label.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>(), sha.chars().take(8).collect::<String>().to_ascii_lowercase());
    if exists(&run) {
        return Err(format!("{run} was already ingested"));
    }
    let mut title = format!("{label} engine-run report (host {}, target {}", report["host"].as_str().unwrap_or("unknown"), report["target"].as_str().unwrap_or("unknown"));
    if let Some(m) = report["mode"].as_str() {
        title.push_str(&format!(", mode {m}"));
    }
    title.push(')');
    let ev = |kind: &str, ek: &str, eid: &str, data: Value| sink.emit(&run, NewEvent::new(kind, ek, eid, data));
    ev("run_started", "run", &run, json!({"title": title, "state": "running", "origin": "manual"}))?;

    let mut doubles: Vec<Value> = Vec::new();
    for d in report["doubles"].as_array().into_iter().flatten() {
        let (Some(part), Some(status)) = (d["part"].as_str().filter(|x| !x.is_empty()), d["status"].as_str().filter(|x| !x.is_empty())) else {
            return Err("every doubles[] entry needs a part and a status (a stand-in is never dropped)".into());
        };
        let id = format!("{part}:{status}");
        if !doubles.iter().any(|x| x["id"] == id) {
            doubles.push(json!({"id": id, "what": double_what(d)}));
        }
    }
    if !doubles.is_empty() {
        ev("doubles_declared", "run", &run, json!({"doubles": doubles}))?;
    }

    let mut prev: Option<&str> = None;
    for st in steps {
        let id = st["id"].as_str().ok_or("every step needs an id")?;
        let status = st["status"].as_str().unwrap_or("unknown");
        let node = step_node(id, status, prev);
        ev("node_status_changed", "node", id, json!({"node": node}))?;
        prev = Some(id);
    }

    // What the thread committed (when the report carries it) fills the panels through the very projection the live stream uses.
    let panel_events = report.get("committed").filter(|c| c.is_object()).map(|c| crate::panels::project(c, Some(report), &crate::store::now_iso())).unwrap_or_default();
    let has_gates = panel_events.iter().any(|e| e.kind == "gates_set");
    for e in panel_events {
        sink.emit(&run, e)?;
    }
    if !has_gates {
        ev("gates_set", "run", &run, gates_data(report))?;
    }
    ev("run_state_changed", "run", &run, json!({"state": "completed"}))?;
    Ok(run)
}

/// The data the console contract suite reads unconditionally (run `run-active`, decision `dec-1`, proposal `prop-1`), under a run that says
/// plainly it is not an engine run.
pub fn contract_seed(sink: &dyn RunEventSink) -> Result<String, String> {
    let run = "run-active";
    let ev = |kind: &str, ek: &str, eid: &str, data: Value| sink.emit(run, NewEvent::new(kind, ek, eid, data));
    ev("run_started", "run", run, json!({"title": "Contract seed (not an engine run)", "state": "completed", "origin": "manual"}))?;
    ev("doubles_declared", "run", run, json!({"doubles": [{"id": "contract_seed:fixture", "what": "static data so the console contract suite finds decision dec-1 and proposal prop-1; produced by no engine", "until": "the engine writes decisions and proposals"}]}))?;
    for (id, status, dep) in [("scout", "complete", vec![]), ("hypothesis", "running", vec!["scout"])] {
        let node = json!({"node_id": id, "label": format!("{id} [contract_seed]"), "stage": id, "status": status, "depends_on": dep, "reason_code": null, "node_kind": "material_step", "trace_id": null});
        ev("node_status_changed", "node", id, json!({"node": node}))?;
    }
    ev("diff_set", "proposal", "prop-1", json!({"proposal_id": "prop-1", "lines": [
        {"op": "ctx", "text": "retry_policy:"}, {"op": "del", "text": "  max_retries: 2"}, {"op": "add", "text": "  max_retries: 3"},
    ]}))?;
    ev("decision_set", "decision", "dec-1", json!({"decision_id": "dec-1", "available_commands": [], "needs_step_up": true, "domain_revision": 1}))?;
    Ok(run.to_string())
}
