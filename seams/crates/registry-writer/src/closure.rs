//! W13: the closure of a brand-new agent.
//!
//! A new agent is a clone of the donor `consultas` (reasoning `patch::compile_new_agent`: agent, intake flow, two templates). agent-core
//! builds the candidate of a brand-new agent WITHOUT a base release, so the draft must carry the FULL closure of the agent
//! (verified live: with only those four changes `validate` answers REG-PIN for every template slot of the agent and for the
//! `understand` decision model). This module reads those entities, unchanged, from the live registry through the guarded read route and
//! returns them as draft changes (`copies`).
//!
//! Release-level settings (INH1). The `fraude` interrupt, the injection ruleset, language detection and `max_input_chars` of the donor
//! release are not entities of the agent and the engine must never WRITE them (interrupts are admin-gated, D-17). With agent-core
//! PR 51 the draft instead carries `release_settings: {inherit_from: <donor release id>}`: the server copies the donor's settings into
//! the candidate of an agent without a base, so nothing can be removed, weakened or forged by the caller. The SAME reference is in the
//! evaluation drafts and in the announced proposal (what is evaluated is what is announced); the platform human still approves and
//! publishes with step-up. The donor release id is read live (`GET /aliases/consultas/{prod|staging}`, active release only); no
//! published donor release fails closed. Without that field in Core (older Core) the proof says `core_without_inherit_from`, and the
//! old way (explicit settings, evaluation only, labelled `release_settings_assumed`) is used ONLY when the operator configured an
//! explicit admin credential (`Config::admin_settings_fallback`).
use crate::writer::Writer;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// The donor agent whose closure is cloned, and the aliases tried (in order) to find its published release.
pub const DONOR_AGENT: &str = "consultas";
pub const DONOR_ALIASES: [&str; 2] = ["prod", "staging"];

/// Names of the donor release (`consultas-demo`): language detection and injection ruleset the clone cannot work without.
pub const LANGUAGE_DETECTION: &str = "lang-es-pt";
pub const INJECTION_RULESET: &str = "injection-rules";

#[derive(Debug, Clone, PartialEq)]
pub struct Closure {
    /// Unchanged live copies of the entities the clone references (delivered with the proposal).
    pub copies: Vec<Value>,
    /// Evaluation-only drafts (never delivered): with `Settings::Assumed` the injection ruleset entity and the explicit settings.
    pub eval_only: Vec<Value>,
    pub settings: Settings,
    /// With `Settings::Inherit`: the `release_settings {inherit_from}` change, delivered AND evaluated.
    pub inherit: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Settings {
    /// The clone inherits the donor release's settings by server-side reference.
    Inherit { donor_release: String },
    /// Fallback with an explicit admin credential: the donor's settings written explicitly, evaluation only.
    Assumed,
}

impl Settings {
    /// The code the judge input carries (`settings_inherited` vs `release_settings_assumed` assumption).
    pub fn code(&self) -> &'static str {
        match self {
            Settings::Inherit { .. } => "inherit_from",
            Settings::Assumed => "assumed",
        }
    }
}

/// The donor's live release id: the alias `prod`, else `staging`, of `DONOR_AGENT`, active. `Err` is the closed reason.
pub fn donor_release(w: &Writer) -> Result<String, String> {
    for alias in DONOR_ALIASES {
        let Some(st) = w.fetch_alias(DONOR_AGENT, alias) else { continue };
        if st["status"].as_str().is_some_and(|s| s != "active") {
            continue;
        }
        if let Some(id) = st["release_id"].as_str().filter(|r| crate::guard::ok_seg(r)) {
            return Ok(id.to_string());
        }
    }
    Err(format!("donor_without_published_release: agent {DONOR_AGENT} has no active release on alias prod or staging, so the clone has nothing to inherit its settings from"))
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
    donor_closure_with(w, clone_changes, false)
}

/// `assumed`: the explicit-admin fallback (only ever asked for after a Core without `inherit_from`).
pub fn donor_closure_with(w: &Writer, clone_changes: &[Value], assumed: bool) -> Result<Closure, String> {
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
    if !assumed {
        // The server resolves the donor's ruleset by reference, but the entity must be in the draft (REG-PIN): an unchanged copy.
        want.push(("injection_ruleset", INJECTION_RULESET.to_string()));
    }
    let donor = if assumed { None } else { Some(donor_release(w)?) };
    let mut seen = BTreeSet::new();
    let mut copies = vec![];
    for (kind, id) in want {
        if own.contains(&(kind.to_string(), id.clone())) || !seen.insert((kind, id.clone())) {
            continue;
        }
        let live = w.fetch_content(kind, &id).ok_or_else(|| format!("closure: {kind} {id} cannot be read from the registry"))?;
        copies.push(copy_of(kind, live));
    }
    if let Some(donor_release) = donor {
        let settings = json!({"kind": "release_settings", "content": {"inherit_from": donor_release}, "docs": {
            "description": format!("[improvement-engine] the clone inherits the safety settings of the donor release {donor_release} (interrupts, language detection, injection ruleset, max input chars) by server-side reference"),
            "rationale": "The engine never writes release settings: agent-core copies them from the donor release. Approval and publication still need the platform human with step-up.",
            "changelog": "Adds release_settings.inherit_from; no interrupt is written by the engine."}});
        return Ok(Closure { copies, eval_only: vec![], settings: Settings::Inherit { donor_release }, inherit: Some(settings) });
    }
    let ruleset = w.fetch_content("injection_ruleset", INJECTION_RULESET).ok_or_else(|| format!("closure: injection_ruleset {INJECTION_RULESET} cannot be read from the registry"))?;
    let settings = json!({"kind": "release_settings", "content": {
        "interrupts": [{"id": "fraude", "priority": 100, "action": {"type": "escalate", "target_queue": "fraude", "priority": "critical"}}],
        "injection_ruleset": format!("{INJECTION_RULESET}@1"), "language_detection": format!("{LANGUAGE_DETECTION}@1")},
        "docs": {"description": "evaluation-only: the release-level settings of the donor release (human-owned)", "rationale": "The proof evaluates the clone with the donor's settings; the engine never delivers them.", "changelog": "evaluation only"}});
    Ok(Closure { copies, eval_only: vec![copy_of("injection_ruleset", ruleset), settings], settings: Settings::Assumed, inherit: None })
}

/// The changes of ONE evaluation run of a new-agent candidate: the proposal, its closure and the settings (by reference, or the
/// evaluation-only explicit ones of the admin fallback).
pub fn eval_changes(proposal: &[Value], c: &Closure) -> Vec<Value> {
    let mut v: Vec<Value> = proposal.to_vec();
    v.extend(c.copies.iter().cloned());
    v.extend(c.inherit.iter().cloned());
    v.extend(c.eval_only.iter().cloned());
    v
}

/// What is delivered when announced: the proposal, the unchanged closure copies and, by reference, the donor's settings.
pub fn delivered_changes(proposal: &[Value], c: &Closure) -> Vec<Value> {
    let mut v: Vec<Value> = proposal.to_vec();
    v.extend(c.copies.iter().cloned());
    v.extend(c.inherit.iter().cloned());
    v
}

/// The closure parts an announced proposal carries beyond the compiled changes.
pub fn delivered_extra(c: &Closure) -> Vec<Value> {
    let mut v = c.copies.clone();
    v.extend(c.inherit.iter().cloned());
    v
}
