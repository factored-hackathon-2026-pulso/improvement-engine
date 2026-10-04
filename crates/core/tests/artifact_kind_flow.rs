//! Offline conformance tests for the X-ARTIF Flow/compiled-tree module.
//!
//! Keep this path-isolated until the train integrator registers the artifact
//! kind module in `lib.rs`; BKF must not silently claim live Core support.

#[path = "../src/artifact_kind_flow.rs"]
mod artifact_kind_flow;

use artifact_kind_flow::{
    FlowOperation, QueueCall, QueueOrderAssessment, assess_queue_order, compile_flow,
};
use serde_json::{Value, json};

fn minimal_flow(id: &str, outcome: &str) -> Value {
    json!({
        "id": id,
        "version": "1.0.0",
        "priority": 10,
        "nodes": [{
            "id": "done",
            "type": "end",
            "config": { "outcome": outcome }
        }]
    })
}

fn flow_with_serialized_size(size: usize) -> Value {
    let mut flow = minimal_flow("payment_help", "resolved");
    let current_size = serde_json::to_vec(&flow).unwrap().len();
    let current_node_id_size = flow["nodes"][0]["id"].as_str().unwrap().len();
    let target_node_id_size = size - current_size + current_node_id_size;
    flow["nodes"][0]["id"] = json!("a".repeat(target_node_id_size));
    assert_eq!(serde_json::to_vec(&flow).unwrap().len(), size);
    flow
}

#[test]
fn add_flow_compiles_the_four_key_flow_to_the_local_golden() {
    let compiled = compile_flow(
        FlowOperation::Add,
        &minimal_flow("payment_help", "resolved"),
    )
    .expect("a minimal four-key Flow is supported by the offline compiler");

    let expected = json!({
        "operation": "add",
        "kind": "flow",
        "content": {
            "id": "payment_help",
            "version": "1.0.0",
            "priority": 10,
            "nodes": [{
                "id": "done",
                "type": "end",
                "config": { "outcome": "resolved" }
            }]
        }
    });
    assert_eq!(compiled, expected);
}

#[test]
fn replace_flow_compiles_to_a_distinct_offline_golden() {
    let compiled = compile_flow(
        FlowOperation::Replace,
        &minimal_flow("payment_help", "escalated"),
    )
    .expect("a replacement Flow remains within the offline supported shape");

    let expected = json!({
        "operation": "replace",
        "kind": "flow",
        "content": {
            "id": "payment_help",
            "version": "1.0.0",
            "priority": 10,
            "nodes": [{
                "id": "done",
                "type": "end",
                "config": { "outcome": "escalated" }
            }]
        }
    });
    assert_eq!(compiled, expected);
}

#[test]
fn flow_shape_is_closed_and_enforces_the_pinned_core_node_limit() {
    let mut extra_flow_key = minimal_flow("payment_help", "resolved");
    extra_flow_key["description"] = json!("not part of the frozen four-key shape");
    assert!(compile_flow(FlowOperation::Add, &extra_flow_key).is_err());

    let node = json!({
        "id": "done",
        "type": "end",
        "config": { "outcome": "resolved" }
    });
    let at_limit = json!({
        "id": "payment_help",
        "version": "1.0.0",
        "priority": 10,
        "nodes": (0..200).map(|index| {
            let mut node = node.clone();
            node["id"] = json!(format!("done_{index}"));
            node
        }).collect::<Vec<_>>()
    });
    assert!(compile_flow(FlowOperation::Add, &at_limit).is_ok());

    let over_limit = json!({
        "id": "payment_help",
        "version": "1.0.0",
        "priority": 10,
        "nodes": (0..201).map(|index| {
            let mut node = node.clone();
            node["id"] = json!(format!("done_{index}"));
            node
        }).collect::<Vec<_>>()
    });
    assert!(compile_flow(FlowOperation::Add, &over_limit).is_err());
}

#[test]
fn flow_content_at_the_pinned_entity_byte_limit_is_accepted() {
    let flow = flow_with_serialized_size(262_144);

    assert!(compile_flow(FlowOperation::Add, &flow).is_ok());
}

#[test]
fn flow_content_one_byte_over_the_pinned_entity_limit_is_rejected() {
    let flow = flow_with_serialized_size(262_145);

    assert!(compile_flow(FlowOperation::Add, &flow).is_err());
}

#[test]
fn queue_order_is_checked_per_tool_queue_not_by_global_interleaving() {
    let reference = [
        QueueCall::new("lookup", "lookup_primary"),
        QueueCall::new("risk", "risk_check"),
        QueueCall::new("lookup", "lookup_fallback"),
    ];
    let candidate = [
        QueueCall::new("risk", "risk_check"),
        QueueCall::new("lookup", "lookup_primary"),
        QueueCall::new("lookup", "lookup_fallback"),
    ];

    assert_eq!(
        assess_queue_order(&reference, &candidate),
        QueueOrderAssessment::Compatible
    );
}

#[test]
fn queue_order_detects_reordered_calls_to_the_same_tool() {
    let reference = [
        QueueCall::new("lookup", "lookup_primary"),
        QueueCall::new("lookup", "lookup_fallback"),
    ];
    let candidate = [
        QueueCall::new("lookup", "lookup_fallback"),
        QueueCall::new("lookup", "lookup_primary"),
    ];

    assert_eq!(
        assess_queue_order(&reference, &candidate),
        QueueOrderAssessment::OrderSensitive
    );
}

#[test]
fn queue_order_reports_missing_seeded_tool_calls() {
    let reference = [QueueCall::new("lookup", "lookup_primary")];
    let candidate = [
        QueueCall::new("lookup", "lookup_primary"),
        QueueCall::new("balance", "balance_check"),
    ];

    assert_eq!(
        assess_queue_order(&reference, &candidate),
        QueueOrderAssessment::MissingSeed
    );
}
