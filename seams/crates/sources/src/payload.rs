//! EVT2: the payload of the platform events the cells need, cleaned client-side before it reaches a package.
//! Mirror of `PAYLOAD_KEYS` and the closed enums of `scripts/aggregate/platform_event_cells.py` (drift-tested there). Anything not listed
//! (analyst ids, run and trace ids, failure codes, texts) is dropped here, and the aggregator checks every value again.
use serde_json::{Map, Value};

/// event type -> allow-listed payload keys.
pub const PAYLOAD_KEYS: &[(&str, &[&str])] = &[
    ("copilot.suggestion_ready", &["agent", "release"]),
    ("copilot.suggestion_none", &["agent", "release"]),
    ("copilot.suggestion_failed", &[]),
    ("copilot.suggestion_decided", &["subject", "decision", "edit_distance_permille", "agent", "release"]),
    ("copilot.tool_used", &["tool"]),
    ("case.type_changed", &["from", "to"]),
    ("assistant.turn_answered", &["agent", "release"]),
    ("assistant.ended", &["result"]),
    ("case.opened", &[]),
];
const SUBJECTS: &[&str] = &["reply", "escalation"];
const DECISIONS: &[&str] = &["used", "edited", "discarded", "ignored", "accepted"];
const RESULTS: &[&str] = &["resolved", "escalated", "ended", "failed", "released"];
const CASE_TYPES: &[&str] = &["none", "unrecognized_charge", "undue_charge", "app_issue", "branch_service", "service_quality", "virtual_card"];
const ID_PREFIXES: &[&str] = &["case-", "cus-", "cust-", "stf-", "staff-", "evt-", "trn-", "qst-", "ast-", "ana-", "sup-", "agt-", "run-", "tr-"];

pub fn keys_of(event_type: &str) -> Option<&'static [&'static str]> {
    PAYLOAD_KEYS.iter().find(|(t, _)| *t == event_type).map(|(_, k)| *k)
}

/// A bounded identifier-like token that is not a platform id.
fn safe_token(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':' | b'@' | b'+' | b'-'))
        && !ID_PREFIXES.iter().any(|p| s.to_ascii_lowercase().starts_with(p))
}

fn value_ok(key: &str, v: &Value) -> bool {
    match (key, v) {
        ("subject", Value::String(s)) => SUBJECTS.contains(&s.as_str()),
        ("decision", Value::String(s)) => DECISIONS.contains(&s.as_str()),
        ("result", Value::String(s)) => RESULTS.contains(&s.as_str()),
        ("from" | "to", Value::String(s)) => CASE_TYPES.contains(&s.as_str()),
        ("edit_distance_permille", Value::Number(n)) => n.as_i64().is_some_and(|x| (0..=1000).contains(&x)),
        ("agent" | "release" | "tool", Value::String(s)) => safe_token(s),
        _ => false,
    }
}

/// The cleaned payload of an event, or `None` when the type needs none, or the payload is missing or not a JSON object.
pub fn clean(event_type: &str, raw: Option<&str>) -> Option<Value> {
    let keys = keys_of(event_type)?;
    let Value::Object(o) = serde_json::from_str::<Value>(raw?).ok()? else { return None };
    let mut out = Map::new();
    for k in keys {
        if let Some(v) = o.get(*k).filter(|v| value_ok(k, v)) {
            out.insert((*k).to_owned(), v.clone());
        }
    }
    Some(Value::Object(out))
}
