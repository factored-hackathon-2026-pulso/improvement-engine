//! ART2: deterministic compilers for two artifact kinds beyond text patches: a READ-ONLY TOOL LINK and a TIGHTEN-ONLY POLICY draft.
//!
//! Nothing here reads a model answer except the edge id the Builder picks from a menu the engine derives from the real flow. Tool
//! ref, version, args and ToolDef come from the registry/tool-service snapshots, the new threshold from structured params.
//! Closed denial codes: `write_tool_human_only`, `tool_not_in_service`, `source_mismatch`, `source_missing`, `principal_not_served`,
//! `already_linked`, `tool_def_drift`, `edge_unknown`, `unknown_policy_shape`, `direction_unknown`, `policy_loosening_denied`,
//! `variable_changed`.
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    pub code: &'static str,
    pub why: String,
}
fn deny<T>(code: &'static str, why: impl Into<String>) -> Result<T, Denied> {
    Err(Denied { code, why: why.into() })
}
fn bump(v: &str, minor: bool) -> String {
    let p: Vec<u64> = v.split('.').filter_map(|x| x.parse().ok()).collect();
    match (p.len() == 3, minor) {
        (false, _) => v.into(),
        (true, true) => format!("{}.{}.0", p[0], p[1] + 1),
        (true, false) => format!("{}.{}.{}", p[0], p[1], p[2] + 1),
    }
}

// ---------------------------------------------------------------------------------------------------------------- tool link

/// `GET /v1/tools` of the tool-service: the oracle of what really exists. Principal types it serves are customer and advisor.
pub const SERVED_PRINCIPALS: [&str; 2] = ["customer", "advisor"];

#[derive(Debug, Clone)]
pub struct ToolService(Vec<Value>);

impl ToolService {
    pub fn from_json(v: &Value) -> Option<ToolService> {
        Some(ToolService(v.get("tools").unwrap_or(v).as_array()?.clone()))
    }
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.0.iter().find(|t| t["id"] == id)
    }
}

/// Preconditions of a link; `tool_def` is the live registry ToolDef, `agent` the live Agent.
pub fn check_link(tool_def: &Value, svc: &ToolService, agent: &Value) -> Result<(), Denied> {
    let id = tool_def["id"].as_str().unwrap_or("");
    let risk = tool_def["risk_class"].as_str().unwrap_or("");
    if risk != "read" {
        return deny("write_tool_human_only", format!("{id} has risk_class {risk:?}: only read tools can be linked by the engine"));
    }
    let Some(s) = svc.get(id) else { return deny("tool_not_in_service", format!("{id} is not listed by the tool-service")) };
    if s["risk_class"] != tool_def["risk_class"] || s["min_auth_level"] != tool_def["min_auth_level"] {
        return deny("tool_def_drift", format!("{id}: risk_class or min_auth_level differ between registry and tool-service"));
    }
    let src = tool_def["source"].as_str().unwrap_or("");
    if src.is_empty() {
        return deny("source_missing", format!("{id} has no source: the field classifier cannot classify its result"));
    }
    if s["source"].as_str() != Some(src) {
        return deny("source_mismatch", format!("{id}: registry source {src:?} differs from the service source {:?}", s["source"]));
    }
    if tool_def["args_schema"]["required"].as_array().is_some_and(|r| !r.is_empty()) {
        return deny("args_required", format!("{id} has required args: the engine links only tools it can call without arguments"));
    }
    let inv: Vec<&str> = agent["invocable_by"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if inv.is_empty() || !inv.iter().all(|p| SERVED_PRINCIPALS.contains(p)) {
        return deny("principal_not_served", format!("agent invocable_by {inv:?} is not within {SERVED_PRINCIPALS:?}"));
    }
    let r = format!("{id}@{}", tool_def["version"].as_str().unwrap_or("").split('.').next().unwrap_or(""));
    if agent["tools_allowed"].as_array().into_iter().flatten().any(|t| t.as_str() == Some(&r)) {
        return deny("already_linked", format!("{r} is already in tools_allowed"));
    }
    Ok(())
}

/// The positions the Builder may choose: edges whose source is a collect/tool/respond node and whose label is the plain
/// continuation (`ok`/`next`) towards a non-terminal, non-escalate node. Edges leaving rule/decide/confirm/verify/escalate/end are frozen.
pub fn edge_menu(flow: &Value) -> Vec<Value> {
    let nodes = flow["nodes"].as_array().cloned().unwrap_or_default();
    let ty = |id: &str| nodes.iter().find(|n| n["id"] == id).and_then(|n| n["type"].as_str()).unwrap_or("").to_string();
    let mut out = vec![];
    for n in &nodes {
        if !["collect", "tool", "respond"].contains(&n["type"].as_str().unwrap_or("")) {
            continue;
        }
        for (label, to) in n["next"].as_object().into_iter().flatten() {
            let to = to.as_str().unwrap_or("");
            if !["ok", "next"].contains(&label.as_str()) || ["escalate", "end", ""].contains(&ty(to).as_str()) || to == n["id"].as_str().unwrap_or("") {
                continue;
            }
            // a respond that awaits the customer, or that is a loop target, is not a pass-through position
            if n["type"] == "respond" && n["config"]["await"] == true {
                continue;
            }
            out.push(json!({"edge_id": format!("{}.{label}", n["id"].as_str().unwrap_or("")), "from": n["id"], "label": label, "to": to}));
        }
    }
    out
}

/// Inserts a `tool` node on the chosen edge (pass-through: its `ok` goes to the original target; error, timeout and denied go to an
/// EXISTING `tool_failure` escalate node). Also appends the tool to the agent and copies the ToolDef unchanged.
pub fn compile_link_tool(flow: &Value, agent: &Value, tool_def: &Value, svc: &ToolService, edge_id: &str) -> Result<Vec<Value>, Denied> {
    check_link(tool_def, svc, agent)?;
    let menu = edge_menu(flow);
    let Some(edge) = menu.iter().find(|e| e["edge_id"] == edge_id) else { return deny("edge_unknown", format!("{edge_id} is not on the menu")) };
    let nodes = flow["nodes"].as_array().cloned().unwrap_or_default();
    let esc = nodes.iter().find(|n| n["type"] == "escalate" && n["config"]["reason_code"] == "tool_failure").and_then(|n| n["id"].as_str().map(str::to_string));
    let Some(esc) = esc else { return deny("no_error_exit", "the flow has no tool_failure escalate node to wire the error exits to") };
    let (tid, tver) = (tool_def["id"].as_str().unwrap_or(""), tool_def["version"].as_str().unwrap_or(""));
    let major = tver.split('.').next().unwrap_or("1");
    let new_id = format!("eng_link_{tid}");
    if nodes.iter().any(|n| n["id"] == new_id.as_str()) {
        return deny("already_linked", "the flow already has the link node");
    }
    let (from, label, to) = (edge["from"].as_str().unwrap_or(""), edge["label"].as_str().unwrap_or(""), edge["to"].clone());
    let mut new_nodes = vec![];
    for n in &nodes {
        let mut n = n.clone();
        if n["id"] == from {
            n["next"][label] = json!(new_id);
        }
        new_nodes.push(n);
    }
    new_nodes.push(json!({"id": new_id, "type": "tool", "config": {"tool": format!("{tid}@{major}"), "args": {}, "save_as": format!("eng_{tid}")},
                          "next": {"ok": to, "error": esc, "timeout": esc, "denied": esc}}));
    let mut f = flow.clone();
    f["nodes"] = Value::Array(new_nodes);
    f["version"] = json!(bump(flow["version"].as_str().unwrap_or(""), true));
    let mut a = agent.clone();
    let mut allowed = a["tools_allowed"].as_array().cloned().unwrap_or_default();
    allowed.push(json!(format!("{tid}@{major}")));
    a["tools_allowed"] = Value::Array(allowed);
    a["version"] = json!(bump(agent["version"].as_str().unwrap_or(""), false));
    let docs = |d: &str| json!({"description": d, "rationale": "link to an existing read-only tool", "changelog": d});
    Ok(vec![
        json!({"kind": "flow", "content": f, "docs": docs("[improvement-engine] link to existing tool, read-only: pass-through tool node, failures escalate as tool_failure")}),
        json!({"kind": "agent", "content": a, "docs": docs("[improvement-engine] tools_allowed gains the linked read tool")}),
        json!({"kind": "tool", "content": tool_def, "docs": docs("unchanged copy of the live ToolDef (REG-PIN)")}),
    ])
}

// ---------------------------------------------------------------------------------------------------------------- policy

/// Owners the engine may edit without a human acknowledgement. Anything else (or a missing owner) needs `owner_ack`.
pub const ENGINE_OWNER: &str = "engine";

/// `{op: [{var: X}, number]}` as `(op, var, k)`; `None` for any other shape (refused as unknown).
fn comparison(e: &Value) -> Option<(String, String, f64)> {
    let o = e.as_object().filter(|o| o.len() == 1)?;
    let (op, args) = o.iter().next()?;
    let a = args.as_array().filter(|a| a.len() == 2)?;
    let var = a[0].get("var")?.as_str()?;
    Some((op.clone(), var.into(), a[1].as_f64()?))
}

/// Interval of the values that satisfy `x op k` as `(lo, lo_closed, hi, hi_closed)` over the extended reals.
fn interval(op: &str, k: f64) -> Option<(f64, bool, f64, bool)> {
    Some(match op {
        ">" => (k, false, f64::INFINITY, false),
        ">=" => (k, true, f64::INFINITY, false),
        "<" => (f64::NEG_INFINITY, false, k, false),
        "<=" => (f64::NEG_INFINITY, false, k, true),
        _ => return None,
    })
}

/// `new` is NOT WEAKER than `old` iff every value that triggers the old expression triggers the new one (set inclusion of the
/// triggering intervals), over the same variable. Only the single-comparison shape is defined; anything else is refused.
pub fn not_weaker(old: &Value, new: &Value) -> Result<(), Denied> {
    let (Some((oo, ov, ok)), Some((no, nv, nk))) = (comparison(old), comparison(new)) else { return deny("unknown_policy_shape", "only a single numeric comparison {op:[{var},number]} is defined") };
    if ov != nv {
        return deny("variable_changed", "the compared variable differs");
    }
    let (Some(a), Some(b)) = (interval(&oo, ok), interval(&no, nk)) else { return deny("unknown_policy_shape", "operator outside > >= < <=") };
    let lo_ok = b.0 < a.0 || (b.0 == a.0 && (b.1 || !a.1));
    let hi_ok = b.2 > a.2 || (b.2 == a.2 && (b.3 || !a.3));
    if lo_ok && hi_ok { Ok(()) } else { deny("policy_loosening_denied", "the new expression would trigger for fewer values than the old one") }
}

/// Does the TRUE branch of every rule node that consumes `policy_id` lead to an escalate node? (the direction the comparator assumes)
pub fn true_branch_escalates(flow: &Value, policy_id: &str) -> bool {
    let nodes = flow["nodes"].as_array().cloned().unwrap_or_default();
    let mut seen = false;
    for n in nodes.iter().filter(|n| n["type"] == "rule" && n["config"]["policy"].as_str().is_some_and(|p| p.split('@').next() == Some(policy_id))) {
        seen = true;
        let t = n["next"]["true"].as_str().unwrap_or("");
        if !nodes.iter().any(|m| m["id"] == t && m["type"] == "escalate") {
            return false;
        }
    }
    seen
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyDraft {
    pub changes: Vec<Value>,
    /// Human-owned: needs the owner's explicit acknowledgement and is never announced automatically (`needs_owner_ack`).
    pub needs_owner_ack: bool,
    pub old_value: f64,
    pub new_value: f64,
}

/// `tighten_threshold`: the new value is a STRUCTURED param (never free text, never the Builder's). The comparator proves not-weaker.
pub fn tighten_policy(policy: &Value, consumer_flow: &Value, new_value: f64) -> Result<PolicyDraft, Denied> {
    let id = policy["id"].as_str().unwrap_or("");
    let (op, var, old) = comparison(&policy["expr"]).ok_or_else(|| Denied { code: "unknown_policy_shape", why: "only a single numeric comparison is defined".into() })?;
    if !true_branch_escalates(consumer_flow, id) {
        return deny("direction_unknown", "the rule that consumes the policy does not send its true branch to an escalate node");
    }
    let new_expr = json!({ op.clone(): [{"var": var}, new_value] });
    not_weaker(&policy["expr"], &new_expr)?;
    let mut p = policy.clone();
    p["expr"] = new_expr;
    p["version"] = json!(bump(policy["version"].as_str().unwrap_or(""), false));
    let needs = policy["owner"].as_str() != Some(ENGINE_OWNER);
    let mut docs = json!({"description": format!("[improvement-engine] tighten-only change of policy {id}: threshold {old} to {new_value}"), "rationale": "stricter policy: escalates for every value the old one escalated, plus more", "changelog": "threshold moved towards more escalation"});
    if needs {
        docs["owner_ack"] = json!({"required": true, "owner": policy["owner"], "announce": "never_automatic"});
    }
    Ok(PolicyDraft { changes: vec![json!({"kind": "policy", "content": p, "docs": docs})], needs_owner_ack: needs, old_value: old, new_value })
}

/// Boundary values (USD) of the policy window for the human note and the guard scenarios: both sides of the old and new thresholds.
pub fn boundary_values(old: f64, new: f64) -> Vec<f64> {
    let mut v: Vec<f64> = [old, new].iter().flat_map(|t| [t - 1.0, *t, t + 1.0]).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v.dedup();
    v
}

/// `policy_hypothesis` note: no draft; a human decides.
pub fn policy_hypothesis(policy: &Value, registry_value: f64, doc_value: f64) -> Value {
    json!({"outcome": "policy_hypothesis", "draft": null, "policy": policy["id"], "owner": policy["owner"], "registry_threshold": registry_value, "document_threshold": doc_value,
           "window_auto_processed_but_documented_for_a_person": [doc_value, registry_value], "boundary_guards": boundary_values(registry_value, doc_value),
           "decision": "human_owner", "engine_never_picks_the_value": true})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pol(op: &str, k: f64, owner: &str) -> Value {
        json!({"id": "p", "version": "1.0.0", "owner": owner, "expr": {op: [{"var": "facts.m"}, k]}})
    }
    fn flow() -> Value {
        json!({"id": "f", "version": "1.0.0", "nodes": [
            {"id": "a", "type": "collect", "config": {}, "next": {"ok": "t", "max_attempts": "e1"}},
            {"id": "t", "type": "tool", "config": {"tool": "x@1"}, "next": {"ok": "u", "error": "e2"}},
            {"id": "u", "type": "rule", "config": {"policy": "p@1"}, "next": {"true": "e3", "false": "r"}},
            {"id": "r", "type": "respond", "config": {}, "next": {"next": "z"}},
            {"id": "z", "type": "end", "config": {}},
            {"id": "e1", "type": "escalate", "config": {"reason_code": "low_confidence"}},
            {"id": "e2", "type": "escalate", "config": {"reason_code": "tool_failure"}},
            {"id": "e3", "type": "escalate", "config": {"reason_code": "policy:p"}}]})
    }
    fn svc() -> ToolService {
        ToolService::from_json(&json!({"tools": [{"id": "leer", "risk_class": "read", "min_auth_level": "session", "source": "s"},
                                                  {"id": "radicar", "risk_class": "write_reversible", "min_auth_level": "step_up", "source": "s"}]})).unwrap()
    }
    fn tdef(id: &str, risk: &str, src: &str) -> Value {
        json!({"id": id, "version": "1.0.0", "risk_class": risk, "min_auth_level": "session", "source": src})
    }
    fn agent() -> Value {
        json!({"id": "ag", "version": "1.0.0", "invocable_by": ["customer"], "tools_allowed": ["x@1"]})
    }
    #[test]
    fn comparator_is_monotone() {
        assert!(not_weaker(&pol("a", 0.0, "o")["expr"], &pol("a", 0.0, "o")["expr"]).is_err()); // unknown operator
        assert!(not_weaker(&pol(">", 500.0, "o")["expr"], &pol(">", 250.0, "o")["expr"]).is_ok());
        assert!(not_weaker(&pol(">", 500.0, "o")["expr"], &pol(">", 900.0, "o")["expr"]).is_err());
        assert!(not_weaker(&pol(">", 500.0, "o")["expr"], &pol(">=", 500.0, "o")["expr"]).is_ok());
        assert!(not_weaker(&pol(">=", 500.0, "o")["expr"], &pol(">", 500.0, "o")["expr"]).is_err());
        assert!(not_weaker(&pol("<", 10.0, "o")["expr"], &pol("<", 20.0, "o")["expr"]).is_ok());
        assert!(not_weaker(&pol(">", 5.0, "o")["expr"], &pol("<", 9.0, "o")["expr"]).is_err());
        assert_eq!(not_weaker(&json!({"and": [1, 2]}), &json!({"and": [1, 2]})).unwrap_err().code, "unknown_policy_shape");
    }
    #[test]
    fn tighten_marks_human_owned_and_refuses_loosening() {
        let d = tighten_policy(&pol(">", 500.0, "riesgo"), &flow(), 250.0).unwrap();
        assert!(d.needs_owner_ack);
        assert_eq!(d.changes[0]["content"]["version"], "1.0.1");
        assert_eq!(d.changes[0]["docs"]["owner_ack"]["required"], true);
        assert!(!tighten_policy(&pol(">", 500.0, "engine"), &flow(), 250.0).unwrap().needs_owner_ack);
        assert_eq!(tighten_policy(&pol(">", 500.0, "riesgo"), &flow(), 900.0).unwrap_err().code, "policy_loosening_denied");
        let mut f = flow();
        f["nodes"][2]["next"]["true"] = json!("r");
        assert_eq!(tighten_policy(&pol(">", 500.0, "riesgo"), &f, 250.0).unwrap_err().code, "direction_unknown");
    }
    #[test]
    fn menu_freezes_branching_edges_and_link_is_pass_through() {
        let ids: Vec<String> = edge_menu(&flow()).iter().map(|e| e["edge_id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids, vec!["a.ok", "t.ok"]);
        let ch = compile_link_tool(&flow(), &agent(), &tdef("leer", "read", "s"), &svc(), "t.ok").unwrap();
        let nodes = ch[0]["content"]["nodes"].as_array().unwrap();
        let n = nodes.iter().find(|n| n["id"] == "eng_link_leer").unwrap();
        assert_eq!(n["next"]["ok"], "u");
        assert_eq!(n["next"]["error"], "e2");
        assert_eq!(nodes.iter().find(|n| n["id"] == "t").unwrap()["next"]["ok"], "eng_link_leer");
        assert_eq!(nodes.iter().find(|n| n["id"] == "u").unwrap(), &flow()["nodes"][2]); // rule untouched
        assert_eq!(ch[0]["content"]["version"], "1.1.0");
        assert_eq!(ch[1]["content"]["tools_allowed"], json!(["x@1", "leer@1"]));
        assert_eq!(ch[2]["content"], tdef("leer", "read", "s"));
        assert_eq!(compile_link_tool(&flow(), &agent(), &tdef("leer", "read", "s"), &svc(), "u.true").unwrap_err().code, "edge_unknown");
    }
    #[test]
    fn link_preconditions() {
        let c = |t: Value, a: Value| compile_link_tool(&flow(), &a, &t, &svc(), "t.ok").unwrap_err().code;
        assert_eq!(c(tdef("radicar", "write_reversible", "s"), agent()), "write_tool_human_only");
        assert_eq!(c(tdef("nope", "read", "s"), agent()), "tool_not_in_service");
        assert_eq!(c(tdef("leer", "read", "other"), agent()), "source_mismatch");
        assert_eq!(c(tdef("leer", "read", ""), agent()), "source_missing");
        let mut a = agent();
        a["invocable_by"] = json!(["builder"]);
        assert_eq!(c(tdef("leer", "read", "s"), a), "principal_not_served");
    }
    #[test]
    fn hypothesis_has_boundaries() {
        let h = policy_hypothesis(&pol(">", 500.0, "riesgo"), 500.0, 250.0);
        assert_eq!(h["boundary_guards"], json!([249.0, 250.0, 251.0, 499.0, 500.0, 501.0]));
        assert!(h["draft"].is_null());
    }
}
