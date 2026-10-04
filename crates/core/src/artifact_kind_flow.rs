//! Offline Flow/compiled-tree proposal compiler (BKF).
//!
//! This deliberately supports only an offline four-key Flow profile based on
//! the pinned Core shape and the engine's narrow U17 predicate. BK0 still says
//! engine-side Flow proposal is denied, and its only Core validation evidence
//! is a rejection-path test. This module creates a local compiled
//! representation; it does not establish Core acceptance, call Core, validate
//! registry state, execute, evaluate, approve, or publish a Flow.

use std::collections::BTreeMap;

use serde_json::{Value, json};

const MAX_FLOW_NODES: usize = 200;
const MAX_ENTITY_BYTES: usize = 262_144;

/// Offline operations covered by the BKF add/replace goldens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowOperation {
    Add,
    Replace,
}

impl FlowOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Replace => "replace",
        }
    }
}

/// Closed failures from the currently supported offline Flow profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowCompileError {
    InvalidFlow,
    TooManyNodes,
    EntityTooLarge,
}

/// Validate and compile one Flow change to the local BKF wire golden.
///
/// The resulting object is an engine-local representation, not an Agent Core
/// request or a claim that the Core accepts this minimal shape.
pub fn compile_flow(operation: FlowOperation, content: &Value) -> Result<Value, FlowCompileError> {
    validate_flow(content)?;

    let bytes = serde_json::to_vec(content).map_err(|_| FlowCompileError::InvalidFlow)?;
    if bytes.len() > MAX_ENTITY_BYTES {
        return Err(FlowCompileError::EntityTooLarge);
    }

    Ok(json!({
        "operation": operation.as_str(),
        "kind": "flow",
        "content": content,
    }))
}

fn validate_flow(content: &Value) -> Result<(), FlowCompileError> {
    let Some(flow) = content.as_object() else {
        return Err(FlowCompileError::InvalidFlow);
    };
    if flow.len() != 4
        || !["id", "version", "priority", "nodes"]
            .iter()
            .all(|key| flow.contains_key(*key))
    {
        return Err(FlowCompileError::InvalidFlow);
    }

    let valid_flow_id = content
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(is_entity_id);
    let valid_version = content
        .get("version")
        .and_then(Value::as_str)
        .is_some_and(is_semver);
    let valid_priority = content
        .get("priority")
        .and_then(Value::as_i64)
        .is_some_and(|priority| priority >= 0);
    let Some(nodes) = content.get("nodes").and_then(Value::as_array) else {
        return Err(FlowCompileError::InvalidFlow);
    };

    if nodes.is_empty() || !valid_flow_id || !valid_version || !valid_priority {
        return Err(FlowCompileError::InvalidFlow);
    }
    if nodes.len() > MAX_FLOW_NODES {
        return Err(FlowCompileError::TooManyNodes);
    }

    let mut seen_ids = std::collections::BTreeSet::new();
    for node in nodes {
        let Some(node_fields) = node.as_object() else {
            return Err(FlowCompileError::InvalidFlow);
        };
        if node_fields.len() != 3
            || !["id", "type", "config"]
                .iter()
                .all(|key| node_fields.contains_key(*key))
        {
            return Err(FlowCompileError::InvalidFlow);
        }

        let Some(node_id) = node.get("id").and_then(Value::as_str) else {
            return Err(FlowCompileError::InvalidFlow);
        };
        if !is_node_id(node_id) || !seen_ids.insert(node_id) {
            return Err(FlowCompileError::InvalidFlow);
        }
        if node.get("type").and_then(Value::as_str) != Some("end") {
            return Err(FlowCompileError::InvalidFlow);
        }

        let Some(config) = node.get("config").and_then(Value::as_object) else {
            return Err(FlowCompileError::InvalidFlow);
        };
        if config.len() != 1 || !config.contains_key("outcome") {
            return Err(FlowCompileError::InvalidFlow);
        }
        let valid_outcome = config
            .get("outcome")
            .and_then(Value::as_str)
            .is_some_and(is_core_outcome);
        if !valid_outcome {
            return Err(FlowCompileError::InvalidFlow);
        }
    }

    Ok(())
}

fn is_entity_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_/-".contains(&byte)
        })
        && value
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn is_node_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && value
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn is_semver(value: &str) -> bool {
    let mut parts = value.split('.');
    let [Some(major), Some(minor), Some(patch)] = [parts.next(), parts.next(), parts.next()] else {
        return false;
    };
    parts.next().is_none() && [major, minor, patch].into_iter().all(is_semver_number)
}

fn is_semver_number(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
        && value.parse::<u32>().is_ok()
}

fn is_core_outcome(value: &str) -> bool {
    matches!(
        value,
        "resolved"
            | "abstained"
            | "cancelled"
            | "clarify_exhausted"
            | "completed"
            | "failed"
            | "abandoned"
            | "escalated"
            | "transferred"
    )
}

/// One offline trace call-site associated with a tool's seeded FIFO queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueCall<'a> {
    tool_id: &'a str,
    call_site: &'a str,
}

impl<'a> QueueCall<'a> {
    #[must_use]
    pub const fn new(tool_id: &'a str, call_site: &'a str) -> Self {
        Self { tool_id, call_site }
    }
}

/// Static compatibility result for the BKF offline queue-order preflight.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueOrderAssessment {
    Compatible,
    OrderSensitive,
    MissingSeed,
}

/// Compare the call-site order separately for each tool queue.
///
/// Different tools have independent queues, so interleaving them is harmless.
/// A candidate tool with no reference calls has no seed and is rejected. Any
/// changed per-tool call-site sequence is conservatively marked
/// `OrderSensitive`; no candidate is declared safe merely because Agent Core's
/// LocalSandbox repeats a final response. This only analyzes supplied offline
/// traces and never invokes the sandbox or Agent Core.
#[must_use]
pub fn assess_queue_order(
    reference: &[QueueCall<'_>],
    candidate: &[QueueCall<'_>],
) -> QueueOrderAssessment {
    let mut reference_by_tool: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut candidate_by_tool: BTreeMap<&str, Vec<&str>> = BTreeMap::new();

    for call in reference {
        reference_by_tool
            .entry(call.tool_id)
            .or_default()
            .push(call.call_site);
    }
    for call in candidate {
        candidate_by_tool
            .entry(call.tool_id)
            .or_default()
            .push(call.call_site);
    }

    if candidate_by_tool
        .keys()
        .any(|tool_id| !reference_by_tool.contains_key(tool_id))
    {
        return QueueOrderAssessment::MissingSeed;
    }
    if reference_by_tool != candidate_by_tool {
        return QueueOrderAssessment::OrderSensitive;
    }
    QueueOrderAssessment::Compatible
}
