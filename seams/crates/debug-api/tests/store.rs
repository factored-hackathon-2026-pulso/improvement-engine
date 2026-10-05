use debug_api::{NewEvent, RunEventSink, Store};
use serde_json::json;

fn node(id: &str, status: &str) -> NewEvent {
    NewEvent::new("node_status_changed", "node", id, json!({"node": {"node_id": id, "label": id, "stage": "scout", "status": status, "depends_on": [], "reason_code": null, "node_kind": "material_step", "trace_id": null}}))
}
fn start(title: &str) -> NewEvent {
    NewEvent::new("run_started", "run", "r", json!({"title": title, "state": "running", "origin": "manual"}))
}

#[test]
fn sequence_is_monotonic_per_run_and_independent_across_runs() {
    let s = Store::memory();
    let a1 = s.emit("run-a", start("A")).unwrap();
    let a2 = s.emit("run-a", node("scout", "running")).unwrap();
    let b1 = s.emit("run-b", start("B")).unwrap();
    assert_eq!((a1["sequence"].as_i64(), a2["sequence"].as_i64(), b1["sequence"].as_i64()), (Some(1), Some(2), Some(1)));
    assert_eq!(s.head("run-a"), Some(2));
    assert_eq!(s.runs(), vec!["run-a", "run-b"]);
}

#[test]
fn wire_event_carries_both_console_shapes() {
    let s = Store::memory();
    let e = s.emit("run-a", node("scout", "running")).unwrap();
    assert_eq!(e["run_id"], "run-a");
    assert_eq!(e["run_ref"], "run-a");
    assert_eq!(e["entity_ref"], json!({"kind": "node", "id": "scout"}));
    assert_eq!(e["projection_revision"], e["sequence"]);
    assert_eq!((e["kind"].as_str(), e["event_code"].as_str(), e["status"].as_str(), e["stage"].as_str()), (Some("node_status_changed"), Some("node_status_changed"), Some("running"), Some("scout")));
    let id = e["event_id"].as_str().unwrap();
    assert_eq!(id.len(), 36);
    assert!(e["occurred_at"].as_str().unwrap().ends_with('Z'));
    let again = Store::memory().emit("run-a", node("scout", "running")).unwrap();
    assert_eq!(again["event_id"], e["event_id"], "event ids are deterministic per (run, sequence)");
}

#[test]
fn events_after_returns_the_tail_in_order_with_a_limit() {
    let s = Store::memory();
    for i in 0..5 {
        s.emit("r", node(&format!("n{i}"), "running")).unwrap();
    }
    let seqs = |v: Vec<serde_json::Value>| v.iter().map(|e| e["sequence"].as_i64().unwrap()).collect::<Vec<_>>();
    assert_eq!(seqs(s.events_after("r", 2, 100)), vec![3, 4, 5]);
    assert_eq!(seqs(s.events_after("r", 0, 2)), vec![1, 2]);
    assert!(s.events_after("nope", 0, 10).is_empty());
}

#[test]
fn run_ids_that_are_not_path_safe_are_refused() {
    let s = Store::memory();
    for bad in ["", "../x", "a/b", "a b", &"x".repeat(129)] {
        assert!(s.emit(bad, start("t")).is_err(), "{bad:?}");
    }
}

#[test]
fn projection_folds_run_and_node_events() {
    let s = Store::memory();
    s.emit("r", start("Title")).unwrap();
    s.emit("r", node("scout", "running")).unwrap();
    s.emit("r", node("scout", "complete")).unwrap();
    s.emit("r", node("verify", "planned")).unwrap();
    s.emit("r", NewEvent::new("run_state_changed", "run", "r", json!({"state": "completed"}))).unwrap();
    let st = s.state("r").unwrap();
    assert_eq!(st["run"], json!({"run_id": "r", "title": "Title", "state": "completed", "origin": "manual"}));
    let nodes = st["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2, "a node status change upserts by node_id");
    assert_eq!((nodes[0]["node_id"].as_str(), nodes[0]["status"].as_str()), (Some("scout"), Some("complete")));
    assert!(s.state("nope").is_none());
}

#[test]
fn purge_moves_the_floor_but_keeps_the_projection() {
    let s = Store::memory();
    s.emit("r", start("T")).unwrap();
    for i in 0..4 {
        s.emit("r", node(&format!("n{i}"), "complete")).unwrap();
    }
    s.purge_through("r", 3).unwrap();
    assert_eq!((s.floor("r"), s.head("r")), (Some(3), Some(5)));
    let seqs: Vec<i64> = s.events_after("r", 0, 100).iter().map(|e| e["sequence"].as_i64().unwrap()).collect();
    assert_eq!(seqs, vec![4, 5], "purged history is gone, nothing is invented");
    assert_eq!(s.state("r").unwrap()["nodes"].as_array().unwrap().len(), 4, "the snapshot still has every node");
    assert!(s.purge_through("r", 99).is_err(), "cannot purge past the head");
    let next = s.emit("r", node("n9", "running")).unwrap();
    assert_eq!(next["sequence"], 6);
}

#[test]
fn file_store_survives_reopen_and_keeps_sequence_and_purge() {
    let dir = std::env::temp_dir().join(format!("debug-api-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let s = Store::open(&dir).unwrap();
        s.emit("r", start("T")).unwrap();
        s.emit("r", node("a", "complete")).unwrap();
        s.emit("r", node("b", "complete")).unwrap();
        s.purge_through("r", 1).unwrap();
    }
    let s = Store::open(&dir).unwrap();
    assert_eq!((s.floor("r"), s.head("r")), (Some(1), Some(3)));
    assert_eq!(s.state("r").unwrap()["run"]["title"], "T");
    assert_eq!(s.state("r").unwrap()["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(s.emit("r", node("c", "running")).unwrap()["sequence"], 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_ids_that_alias_on_windows_are_refused() {
    let s = Store::memory();
    for bad in ["con", "NUL", "aux.x", "com1", "lpt9", "a.", "trail..", "a:b"] {
        assert!(s.emit(bad, start("t")).is_err(), "{bad:?} must be refused");
    }
}

fn call(ev: &str, role: &str, n: u64) -> serde_json::Value {
    json!({"schema": "pulso.model_call/1", "evidence_ref": ev, "role": role, "n": n, "model_id": "xiaomi/mimo-v2.6-flash", "tokens_in": 10, "tokens_out": 5, "request": {"messages": []}, "response": "r"})
}

#[test]
fn model_calls_are_kept_per_run_deduplicated_validated_bounded_and_survive_reopen() {
    let dir = std::env::temp_dir().join(format!("debug-api-calls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let s = Store::open(&dir).unwrap();
        assert!(s.record_model_calls("r", &[call("ev_1", "scout", 1)]).is_err(), "unknown run");
        s.emit("r", start("T")).unwrap();
        assert_eq!(s.record_model_calls("r", &[call("ev_1", "scout", 1), call("ev_1", "verifier", 1)]).unwrap(), 2);
        assert_eq!(s.record_model_calls("r", &[call("ev_1", "scout", 1), call("ev_2", "scout", 1)]).unwrap(), 1, "the same (evidence, role, n) is recorded once");
        for bad in [json!({"schema": "other/1", "evidence_ref": "e", "role": "scout", "n": 1}), json!({"schema": "pulso.model_call/1", "role": "scout", "n": 1}), json!("x")] {
            assert!(s.record_model_calls("r", &[bad]).is_err());
        }
        let big = json!({"schema": "pulso.model_call/1", "evidence_ref": "e", "role": "scout", "n": 9, "response": "x".repeat(300 * 1024)});
        assert!(s.record_model_calls("r", &[big]).is_err(), "bounded size");
        assert_eq!(s.model_calls("r").len(), 3);
    }
    let s = Store::open(&dir).unwrap();
    let c = s.model_calls("r");
    assert_eq!(c.len(), 3);
    assert_eq!(c[0]["role"], "scout");
    assert_eq!(s.runs(), vec!["r".to_string()], "the calls file is not mistaken for a run");
    assert_eq!(s.record_model_calls("r", &[call("ev_1", "scout", 1)]).unwrap(), 0, "dedup also across a reopen");
    let _ = std::fs::remove_dir_all(&dir);
}
