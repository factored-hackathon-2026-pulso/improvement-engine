//! W13: the closure of a brand-new agent.
//!
//! A new agent is a clone of the donor `consultas` (reasoning `patch::compile_new_agent`: agent, intake flow, two templates). agent-core
//! builds the candidate of a brand-new agent WITHOUT a base release, so the draft must carry the FULL closure of the agent
//! (verified live: with only those four changes `validate` answers REG-PIN for every template slot of the agent and for the
//! `understand` decision model). This module reads those entities, unchanged, from the live registry through the guarded read route and
//! returns them as draft changes (`copies`).
//!
//! Two release-level facts of the donor release are NOT entities of the agent and cannot be proposed by the engine
//! (`release_settings` is human-owned; `Writer::prepare` refuses it): the `fraude` interrupt and the injection ruleset. Verified live:
//! without them agent-core cannot even evaluate the clone (`evaluate` answers 500: the `understand` decision model has no askable
//! field in a release with neither interrupts nor more than one flow). So the PROOF evaluates the clone WITH the donor's settings
//! (`eval_only`, never delivered) and says so (`coverage.assumptions: release_settings_assumed`): the human admin adds exactly those
//! before prod. What the engine delivers is `compiled changes + copies + eval_suite`.
use crate::writer::Writer;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Names of the donor release (`consultas-demo`): language detection and injection ruleset the clone cannot work without.
pub const LANGUAGE_DETECTION: &str = "lang-es-pt";
pub const INJECTION_RULESET: &str = "injection-rules";

#[derive(Debug, Clone, PartialEq)]
pub struct Closure {
    /// Unchanged live copies of the entities the clone references (delivered with the proposal).
    pub copies: Vec<Value>,
    /// Evaluation-only drafts: the injection ruleset entity and the `release_settings` of the donor release (never delivered).
    pub eval_only: Vec<Value>,
}

fn ref_id(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.split('@').next().unwrap_or(s).to_string()),
        Value::Object(o) => o.get("id").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

fn kind_and_id(c: &Value) -> Option<(String, String)> {
    Some((c["kind"].as_str()?.to_string(), c["content"]["id"].as_str()?.to_string()))
}

fn copy_of(kind: &str, content: Value) -> Value {
    json!({"kind": kind, "content": content, "docs": {
        "description": "[improvement-engine] closure copy of the donor consultas, unchanged (a brand-new agent needs its full closure in the draft)",
        "rationale": "Required by agent-core for an agent without a base release; identical to the live entity.",
        "changelog": "Unchanged copy of the live entity."}})
}

/// The entities the cloned agent references and that are not drafted by the clone itself. `Err` names the reference that could not be
/// read (the proof then reports `suite_error`, nothing is announced).
pub fn donor_closure(w: &Writer, clone_changes: &[Value]) -> Result<Closure, String> {
    let agent = clone_changes.iter().find(|c| c["kind"] == "agent").ok_or("the new-agent proposal has no agent change")?;
    let own: BTreeSet<(String, String)> = clone_changes.iter().filter_map(kind_and_id).collect();
    let content = &agent["content"];
    let mut want: Vec<(&str, String)> = vec![];
    for v in content["templates"].as_object().into_iter().flat_map(|m| m.values()) {
        want.extend(ref_id(v).map(|id| ("template", id)));
    }
    want.extend(ref_id(&content["understand"]).map(|id| ("decision_model", id)));
    for v in content["tools_allowed"].as_array().into_iter().flatten() {
        want.extend(ref_id(v).map(|id| ("tool", id)));
    }
    want.push(("language_detection", LANGUAGE_DETECTION.to_string()));
    let mut seen = BTreeSet::new();
    let mut copies = vec![];
    for (kind, id) in want {
        if own.contains(&(kind.to_string(), id.clone())) || !seen.insert((kind, id.clone())) {
            continue;
        }
        let live = w.fetch_content(kind, &id).ok_or_else(|| format!("closure: {kind} {id} cannot be read from the registry"))?;
        copies.push(copy_of(kind, live));
    }
    let ruleset = w.fetch_content("injection_ruleset", INJECTION_RULESET).ok_or_else(|| format!("closure: injection_ruleset {INJECTION_RULESET} cannot be read from the registry"))?;
    let settings = json!({"kind": "release_settings", "content": {
        "interrupts": [{"id": "fraude", "priority": 100, "action": {"type": "escalate", "target_queue": "fraude", "priority": "critical"}}],
        "injection_ruleset": format!("{INJECTION_RULESET}@1"), "language_detection": format!("{LANGUAGE_DETECTION}@1")},
        "docs": {"description": "evaluation-only: the release-level settings of the donor release (human-owned)", "rationale": "The proof evaluates the clone with the donor's settings; the engine never delivers them.", "changelog": "evaluation only"}});
    Ok(Closure { copies, eval_only: vec![copy_of("injection_ruleset", ruleset), settings] })
}

/// The changes of ONE evaluation run of a new-agent candidate: the proposal, its closure and the evaluation-only settings.
pub fn eval_changes(proposal: &[Value], c: &Closure) -> Vec<Value> {
    let mut v: Vec<Value> = proposal.to_vec();
    v.extend(c.copies.iter().cloned());
    v.extend(c.eval_only.iter().cloned());
    v
}

/// What is delivered when announced: the proposal and the unchanged closure copies (no release-level settings).
pub fn delivered_changes(proposal: &[Value], c: &Closure) -> Vec<Value> {
    let mut v: Vec<Value> = proposal.to_vec();
    v.extend(c.copies.iter().cloned());
    v
}
