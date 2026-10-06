//! FLOW1: deterministic compilers for ADDITIVE edits of an agent-core flow (registry kind `flow`).
//!
//! Four ops, in the order of value over risk (decision table: `docs/dev/FLOW_EDITS.md`):
//! `add_validator` (a typed input check on an existing `collect`), `insert_ask` (one more clarifying `collect` on a continuation edge),
//! `insert_notice` (a plain `respond` between a failing edge and its EXISTING escalation) and `insert_ack` (a plain `respond` on a
//! continuation edge). The model never writes a node or an edge: it picks a POSITION id from a menu derived from the real flow graph and a
//! PRESET id from the mapping data (reviewed es/pt text, or a validator). Everything else is derived here.
//!
//! What a compiled edit may do, and what is checked again on the result by [`check_invariants`] (the deterministic recompute):
//! * only add nodes (`respond` with a template, or `collect` whose exhausted attempts go to an existing `low_confidence` escalation) or add a
//!   `validator` to a `collect` that has none; nothing is deleted, no node changes type;
//! * every `rule`, `decide`, `confirm`, `verify`, `escalate`, `end`, write `tool`, `transfer`, `agent`, `suggest`, `knowledge`, `subflow` and
//!   `await_approval` node is byte-identical to the base, and so is every edge that LEAVES one of them (frozen);
//! * contracting the new nodes gives back exactly the base graph (pass-through): every old edge still reaches the same old node;
//! * every node stays reachable, every failure branch that was a safe exit stays one (G0-06), the node count stays bounded.
//!
//! Denial codes: `op_unknown`, `invalid_graph`, `position_unknown`, `edge_frozen`, `position_not_allowed`, `node_protected`,
//! `validator_exists`, `validator_invalid`, `preset_invalid`, `text_not_clean`, `translation_copy`, `slot_taken`, `no_error_exit`,
//! `already_applied`, `edit_limit`, `node_limit`, `template_taken`, `flow_unsupported`, `invariant_broken`.
use crate::art2::{Denied, ref_id};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn deny<T>(code: &'static str, why: impl Into<String>) -> Result<T, Denied> {
    Err(Denied { code, why: why.into() })
}

pub const OPS: [&str; 4] = ["add_validator", "insert_ask", "insert_notice", "insert_ack"];
/// Nodes of a flow after the edit (agent-core caps a flow at 200; the engine stays far below).
pub const NODE_CAP: usize = 60;
/// Nodes the engine itself added to one flow (id prefix `eng_`), over all its versions.
pub const MAX_ENGINE_NODES: usize = 3;
const ENG: &str = "eng_";
/// Node kinds whose body and outgoing edges the engine never touches.
const PROTECTED: [&str; 13] = ["rule", "decide", "confirm", "verify", "escalate", "end", "transfer", "await_approval", "subflow", "knowledge", "agent", "suggest", "tool_write"];
/// Reason codes of an escalation that a notice may precede: the plain failure exits. Policy, fraud and verification exits are not on the menu.
const NOTICE_REASONS: [&str; 2] = ["tool_failure", "low_confidence"];

fn nodes(flow: &Value) -> Vec<Value> {
    flow["nodes"].as_array().cloned().unwrap_or_default()
}
fn ty(n: &Value) -> &str {
    n["type"].as_str().unwrap_or("")
}
fn id(n: &Value) -> &str {
    n["id"].as_str().unwrap_or("")
}
fn is_write(n: &Value) -> bool {
    ty(n) == "tool" && (n["config"].get("action_from").is_some() || n["config"]["draft"] == true)
}
fn kind_of(n: &Value) -> &str {
    if is_write(n) { "tool_write" } else { ty(n) }
}
fn awaits(n: &Value) -> bool {
    ty(n) == "respond" && n["config"]["await"] == true
}
fn get<'a>(ns: &'a [Value], i: &str) -> Option<&'a Value> {
    ns.iter().find(|n| id(n) == i)
}
fn next_of(n: &Value) -> Vec<(String, String)> {
    n["next"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()
}
/// The label by which a node of this kind simply continues, if it has one an insertion may sit on.
fn continuation(n: &Value) -> Option<&'static str> {
    match kind_of(n) {
        "collect" | "tool" => Some("ok"),
        "respond" if !awaits(n) => Some("next"),
        _ => None,
    }
}

fn bump(v: &str, minor: bool) -> String {
    let p: Vec<u64> = v.split('.').filter_map(|x| x.parse().ok()).collect();
    match (p.len() == 3, minor) {
        (false, _) => v.into(),
        (true, true) => format!("{}.{}.0", p[0], p[1] + 1),
        (true, false) => format!("{}.{}.{}", p[0], p[1], p[2] + 1),
    }
}

fn reachable(ns: &[Value]) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<String> = ns.first().map(|n| vec![id(n).to_string()]).unwrap_or_default();
    while let Some(i) = stack.pop() {
        if !seen.insert(i.clone()) {
            continue;
        }
        if let Some(n) = get(ns, &i) {
            stack.extend(next_of(n).into_iter().map(|(_, t)| t));
        }
    }
    seen
}

/// Shape of the base graph the engine is willing to edit. Anything else is `invalid_graph`: the engine does not repair flows.
pub fn validate_graph(flow: &Value) -> Result<(), Denied> {
    let ns = nodes(flow);
    if ns.is_empty() {
        return deny("invalid_graph", "the flow has no nodes");
    }
    let mut ids = BTreeSet::new();
    for n in &ns {
        let i = id(n);
        if i.is_empty() || !i.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return deny("invalid_graph", format!("node id {i:?} is not a valid node id"));
        }
        if !ids.insert(i.to_string()) {
            return deny("invalid_graph", format!("duplicate node id {i}"));
        }
        if ty(n).is_empty() {
            return deny("invalid_graph", format!("node {i} has no type"));
        }
    }
    for n in &ns {
        for (label, t) in next_of(n) {
            if !ids.contains(&t) {
                return deny("invalid_graph", format!("{}.{label} points to {t}, which does not exist", id(n)));
            }
        }
        let labels: BTreeSet<String> = next_of(n).into_iter().map(|(l, _)| l).collect();
        let want: Option<&[&str]> = match kind_of(n) {
            "collect" => Some(&["max_attempts", "ok"]),
            "tool" => Some(&["denied", "error", "ok", "timeout"]),
            "rule" => Some(&["false", "true"]),
            "respond" => Some(&["next"]),
            _ => None,
        };
        if let Some(w) = want
            && labels != w.iter().map(|s| s.to_string()).collect()
        {
            return deny("invalid_graph", format!("node {} ({}) has the outputs {labels:?}, expected {w:?}", id(n), kind_of(n)));
        }
    }
    if let Some(n) = ns.iter().find(|n| ty(n) == "suggest" || (is_write(n) && n["config"]["draft"] == true)) {
        return deny("flow_unsupported", format!("node {} ({}): task and builder flows are not edited by the engine", id(n), kind_of(n)));
    }
    let reach = reachable(&ns);
    if let Some(n) = ns.iter().find(|n| !reach.contains(id(n))) {
        return deny("invalid_graph", format!("node {} is not reachable from the entry", id(n)));
    }
    if flow["version"].as_str().is_none_or(|v| v.split('.').filter_map(|x| x.parse::<u64>().ok()).count() != 3) {
        return deny("invalid_graph", "the flow version is not major.minor.patch");
    }
    Ok(())
}

/// The nodes from the entry to `node` (inclusive) when that path is unique and made only of collect, read tool and plain respond nodes
/// joined by their continuation edge: a scripted scenario can reach `node` deterministically, with seeded tool replies.
fn prefix_chain(ns: &[Value], node: &str) -> Option<Vec<Value>> {
    let entry = id(ns.first()?).to_string();
    let mut chain = vec![];
    let mut cur = node.to_string();
    for _ in 0..ns.len() {
        let n = get(ns, &cur)?.clone();
        if !["collect", "tool", "respond"].contains(&kind_of(&n)) || awaits(&n) {
            return None;
        }
        chain.push(n);
        if cur == entry {
            chain.reverse();
            return Some(chain);
        }
        let mut preds: Vec<(String, String)> = vec![];
        for p in ns {
            for (l, t) in next_of(p) {
                if t == cur && id(p) != cur {
                    preds.push((id(p).to_string(), l));
                }
            }
        }
        let [(p, l)] = preds.as_slice() else { return None };
        let pn = get(ns, p)?;
        if continuation(pn) != Some(l.as_str()) {
            return None;
        }
        cur = p.clone();
    }
    None
}

fn chain_json(chain: &[Value]) -> Value {
    Value::Array(chain.iter().map(|n| json!({"id": id(n), "type": ty(n), "slot": n["config"]["slot"], "tool": n["config"]["tool"].as_str().map(|_| ref_id(&n["config"]["tool"])).or_else(|| n["config"]["tool"].is_object().then(|| ref_id(&n["config"]["tool"])))})).collect())
}

/// What follows a position on the happy path, as far as a scripted scenario can tell: a read `tool`, the run closing, or anything else.
fn after_path(ns: &[Value], start: &str) -> &'static str {
    let mut cur = start.to_string();
    for _ in 0..=ns.len() {
        let Some(n) = get(ns, &cur) else { return "other" };
        match kind_of(n) {
            "tool" => return "tool",
            "end" => return "closes",
            "respond" if !awaits(n) => match n["next"]["next"].as_str() {
                Some(t) => cur = t.to_string(),
                None => return "other",
            },
            _ => return "other",
        }
    }
    "other"
}

/// How native evaluation tells the edited flow from the base at a position reached by `chain`: the candidate stops before a tool call that the
/// base makes (`tool_not_called`) or before the run closes (`run_not_closed`). `None`: nothing in the events differs (no native evidence).
fn discriminator(ns: &[Value], chain: &[Value], to: &str) -> Option<&'static str> {
    let chain_has_tool = chain.iter().any(|n| kind_of(n) == "tool");
    match after_path(ns, to) {
        "tool" if !chain_has_tool => Some("tool_not_called"),
        "closes" => Some("run_not_closed"),
        _ => None,
    }
}

/// The positions the Builder may choose for `op`, derived from the real graph. Each item: `position_id`, the edge or node, node types, and
/// `native_evidence` (agent-core's native evaluation can tell the edited flow from the base on this position, see `chain`).
pub fn menu(op: &str, flow: &Value) -> Vec<Value> {
    let ns = nodes(flow);
    let mut out = vec![];
    match op {
        "insert_ack" | "insert_ask" => {
            for n in &ns {
                let Some(label) = continuation(n) else { continue };
                let Some(to) = n["next"][label].as_str() else { continue };
                let Some(t) = get(&ns, to) else { continue };
                // never in front of a terminal, a verify (only a write reaches it) or a capture_start collect (G0-29 keeps its chain)
                if ["escalate", "end", "verify"].contains(&ty(t)) || to == id(n) || (ty(t) == "collect" && t["config"]["capture_start"] == true) {
                    continue;
                }
                let chain = if op == "insert_ask" { prefix_chain(&ns, id(n)) } else { None };
                let disc = chain.as_deref().and_then(|c| discriminator(&ns, c, to));
                out.push(json!({"position_id": format!("{}.{label}", id(n)), "op": op, "from": id(n), "label": label, "to": to, "from_type": ty(n), "to_type": ty(t),
                                "native_evidence": disc.is_some(), "discriminator": disc, "chain": chain.as_deref().map(chain_json).unwrap_or(Value::Null)}));
            }
        }
        "insert_notice" => {
            for n in &ns {
                let labels: &[&str] = match kind_of(n) {
                    "tool" => &["error", "timeout", "denied"],
                    "collect" => &["max_attempts"],
                    _ => continue,
                };
                for label in labels {
                    let Some(to) = n["next"][*label].as_str() else { continue };
                    let Some(t) = get(&ns, to) else { continue };
                    if ty(t) != "escalate" || !NOTICE_REASONS.contains(&t["config"]["reason_code"].as_str().unwrap_or("")) {
                        continue;
                    }
                    out.push(json!({"position_id": format!("{}.{label}", id(n)), "op": op, "from": id(n), "label": label, "to": to, "from_type": ty(n), "to_type": "escalate",
                                    "reason_code": t["config"]["reason_code"], "native_evidence": false, "discriminator": Value::Null, "chain": Value::Null}));
                }
            }
        }
        "add_validator" => {
            for n in ns.iter().filter(|n| kind_of(n) == "collect" && n["config"].get("validator").is_none_or(Value::is_null)) {
                let chain = prefix_chain(&ns, id(n));
                let to = n["next"]["ok"].as_str().unwrap_or("");
                let disc = chain.as_deref().and_then(|c| discriminator(&ns, &c[..c.len() - 1], to));
                out.push(json!({"position_id": id(n), "op": op, "from": id(n), "label": Value::Null, "to": Value::Null, "from_type": "collect", "to_type": Value::Null,
                                "slot": n["config"]["slot"], "native_evidence": disc.is_some(), "discriminator": disc, "chain": chain.as_deref().map(chain_json).unwrap_or(Value::Null)}));
            }
        }
        _ => {}
    }
    out
}

/// The menu as the lines the Builder reads: `<position id> | <from> -> <to> | evidence`.
pub fn menu_lines(op: &str, flow: &Value) -> String {
    menu(op, flow)
        .iter()
        .map(|m| format!("{} | {} -> {} | native_evidence {}", m["position_id"].as_str().unwrap_or(""), m["from"].as_str().unwrap_or(""), m["to"].as_str().unwrap_or("(this collect)"), m["native_evidence"]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A reviewed text pair (or validator) from the mapping data.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub id: String,
    pub es: String,
    pub pt: String,
    /// `insert_ask`: the name of the new slot.
    pub slot: Option<String>,
    /// `add_validator`: `{kind, value}` of an agent-core `SlotValidator`.
    pub validator: Option<Value>,
}

impl Preset {
    pub fn from_json(v: &Value) -> Result<Preset, String> {
        let s = |k: &str| v[k].as_str().map(str::to_string);
        let id = s("id").filter(|i| !i.is_empty()).ok_or("preset without id")?;
        Ok(Preset { id, es: s("es").unwrap_or_default(), pt: s("pt").unwrap_or_default(), slot: s("slot"), validator: v.get("validator").filter(|x| x.is_object()).cloned() })
    }
}

/// The checks every preset passes, at load time of the mapping table and again in the compiler.
pub fn check_preset(op: &str, p: &Preset) -> Result<(), Denied> {
    if !OPS.contains(&op) {
        return deny("op_unknown", format!("{op:?} is not one of {OPS:?}"));
    }
    if op == "add_validator" {
        let Some(v) = p.validator.as_ref() else { return deny("preset_invalid", format!("preset {} has no validator", p.id)) };
        return check_validator(v);
    }
    check_texts(p)?;
    if op == "insert_ask" {
        let slot = p.slot.clone().unwrap_or_default();
        if slot.is_empty() || !slot.starts_with(|c: char| c.is_ascii_lowercase()) || !slot.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') || slot.len() > 40 {
            return deny("preset_invalid", format!("preset {}: slot {slot:?} is not a valid slot name", p.id));
        }
    }
    Ok(())
}

fn check_texts(p: &Preset) -> Result<(), Denied> {
    for (loc, t) in [("es", &p.es), ("pt", &p.pt)] {
        crate::clean_text(t, 240).map_err(|e| Denied { code: "text_not_clean", why: format!("preset {} {loc}: {e}", p.id) })?;
        if t.chars().count() < 8 || t.contains(['{', '}']) {
            return deny("text_not_clean", format!("preset {} {loc}: too short or carries a brace (a template of this edit has no placeholder)", p.id));
        }
    }
    if p.es.trim().eq_ignore_ascii_case(p.pt.trim()) {
        return deny("translation_copy", format!("preset {}: the pt text equals the es text", p.id));
    }
    Ok(())
}

/// Syntax-level checks of a slot validator (agent-core re-validates at freeze; the accept and reject examples are run by the suite builder).
pub fn check_validator(v: &Value) -> Result<(), Denied> {
    let bad = |why: &str| deny("validator_invalid", why.to_string());
    let kind = v["kind"].as_str().unwrap_or("");
    match kind {
        "type" => {
            if !["string", "integer", "decimal"].contains(&v["value"].as_str().unwrap_or("")) {
                return bad("type must be string, integer or decimal");
            }
        }
        "enum" => {
            let items: Vec<&str> = v["value"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let n = v["value"].as_array().map_or(0, Vec::len);
            if items.is_empty() || items.len() != n || items.iter().collect::<BTreeSet<_>>().len() != n {
                return bad("enum must be a non-empty list of distinct strings");
            }
        }
        "regex" | "extract" => {
            let Some(re) = v["value"].as_str().filter(|r| !r.is_empty() && r.len() <= 200) else { return bad("regex must be a string of at most 200 characters") };
            // nested unbounded quantifiers (catastrophic backtracking) are refused, like agent-core does
            let mut stack: Vec<bool> = vec![];
            let (mut groups, mut prev_close_quant) = (0usize, false);
            let cs: Vec<char> = re.chars().collect();
            let mut i = 0;
            while i < cs.len() {
                match cs[i] {
                    '\\' => i += 1,
                    '(' => {
                        if cs.get(i + 1) != Some(&'?') {
                            groups += 1;
                        }
                        stack.push(false);
                    }
                    ')' => {
                        prev_close_quant = stack.pop().unwrap_or(false);
                        if prev_close_quant && matches!(cs.get(i + 1), Some('*' | '+' | '{')) {
                            return bad("nested unbounded quantifiers");
                        }
                        if prev_close_quant && let Some(top) = stack.last_mut() {
                            *top = true;
                        }
                    }
                    '*' | '+' => {
                        if let Some(top) = stack.last_mut() {
                            *top = true;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            if !stack.is_empty() {
                return bad("unbalanced parentheses");
            }
            if kind == "extract" && groups != 1 {
                return bad("extract needs exactly one capture group");
            }
        }
        _ => return bad("kind must be extract, regex, enum or type"),
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    /// Agent-core draft changes `{kind, content, docs}`: the new flow version, the agent (pin and patch bump) and the new templates.
    pub changes: Vec<Value>,
    /// Structured facts recomputed from base and result (what the independent review sees, never model text).
    pub facts: Value,
    pub node_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Object,
    AtMajor,
    Bare,
}

fn style_of(ns: &[Value]) -> Style {
    for n in ns {
        for k in ["template_ref", "prompt_ref"] {
            let r = &n["config"][k];
            if r.is_object() {
                return Style::Object;
            }
            if let Some(s) = r.as_str() {
                return if s.contains('@') { Style::AtMajor } else { Style::Bare };
            }
        }
    }
    Style::Bare
}

fn mk_ref(style: Style, tid: &str, version: &str) -> Value {
    match style {
        Style::Object => json!({"id": tid, "spec": version}),
        Style::AtMajor => json!(format!("{tid}@{}", version.split('.').next().unwrap_or("1"))),
        Style::Bare => json!(tid),
    }
}

fn slug(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect()
}

/// Why `position` is not on the menu of `op`: a precise closed code.
fn classify_missing(op: &str, ns: &[Value], position: &str) -> Denied {
    let d = |code, why: String| Denied { code, why };
    if op == "add_validator" {
        return match get(ns, position) {
            None => d("position_unknown", format!("{position} is not a node of the flow")),
            Some(n) if kind_of(n) != "collect" => d("node_protected", format!("{position} is a {} node: only a collect takes a validator", kind_of(n))),
            Some(_) => d("validator_exists", format!("{position} already has a validator: the engine adds one, it does not replace one")),
        };
    }
    let Some((from, label)) = position.rsplit_once('.') else { return d("position_unknown", format!("{position} is not <node>.<label>")) };
    let Some(n) = get(ns, from) else { return d("position_unknown", format!("{from} is not a node of the flow")) };
    let Some(to) = n["next"][label].as_str() else { return d("position_unknown", format!("{from} has no output {label}")) };
    if PROTECTED.contains(&kind_of(n)) {
        return d("edge_frozen", format!("{position} leaves a {} node: edges leaving rule, decide, confirm, verify, escalate, end and write nodes are frozen", kind_of(n)));
    }
    d("position_not_allowed", format!("{position} (to {to}) is not a position for {op}: an await respond, a terminal, verify or capture_start target, or a different edge class"))
}

fn engine_nodes(ns: &[Value]) -> usize {
    ns.iter().filter(|n| id(n).starts_with(ENG)).count()
}

/// Compiles one edit. `taken` says whether a template id already exists in the registry (new ids must be free). The result carries the
/// flow (minor bump), the agent (patch bump and exact pin follows the flow) and, for ack/ask/notice, one new template (es and pt).
pub fn compile_flow_edit(flow: &Value, agent: &Value, op: &str, position: &str, preset: &Preset, taken: &dyn Fn(&str) -> bool) -> Result<Edit, Denied> {
    if !OPS.contains(&op) {
        return deny("op_unknown", format!("{op:?} is not one of {OPS:?}"));
    }
    validate_graph(flow)?;
    let ns = nodes(flow);
    let items = menu(op, flow);
    let Some(item) = items.iter().find(|m| m["position_id"] == position) else { return Err(classify_missing(op, &ns, position)) };
    check_preset(op, preset)?;
    if engine_nodes(&ns) >= MAX_ENGINE_NODES && op != "add_validator" {
        return deny("edit_limit", format!("the flow already carries {MAX_ENGINE_NODES} engine-added nodes"));
    }
    if ns.len() + 1 > NODE_CAP {
        return deny("node_limit", format!("the flow would exceed {NODE_CAP} nodes"));
    }
    let from = item["from"].as_str().unwrap_or("").to_string();
    let label = item["label"].as_str().unwrap_or("").to_string();
    let flow_id = flow["id"].as_str().unwrap_or("");
    let new_version = bump(flow["version"].as_str().unwrap_or(""), true);
    let style = style_of(&ns);
    let mut new_nodes: Vec<Value> = ns.clone();
    let mut templates: Vec<Value> = vec![];
    let mut node_id: Option<String> = None;

    if op == "add_validator" {
        let v = preset.validator.clone().unwrap_or(Value::Null);
        let n = new_nodes.iter_mut().find(|n| id(n) == from).ok_or(Denied { code: "position_unknown", why: from.clone() })?;
        n["config"]["validator"] = json!({"kind": v["kind"], "value": v["value"]});
    } else {
        let short = match op {
            "insert_ask" => "ask",
            "insert_notice" => "notice",
            _ => "ack",
        };
        let nid = format!("{ENG}{short}_{from}_{}", slug(&label));
        let tid = format!("t/{nid}_{}", slug(flow_id));
        if ns.iter().any(|n| id(n) == nid) {
            return deny("already_applied", format!("{nid} already exists in the flow"));
        }
        if taken(&tid) {
            return deny("template_taken", format!("the template id {tid} already exists in the registry"));
        }
        let to = item["to"].as_str().unwrap_or("").to_string();
        let tref = mk_ref(style, &tid, "1.0.0");
        let node = match op {
            "insert_ask" => {
                let slot = preset.slot.clone().unwrap_or_default();
                let used = ns.iter().any(|n| ty(n) == "collect" && n["config"]["slot"] == slot.as_str()) || agent["input_schema"].get(&slot).is_some() || agent["accepts"]["slots"].get(&slot).is_some();
                if used {
                    return deny("slot_taken", format!("the slot {slot} already exists"));
                }
                let esc = ns.iter().find(|n| ty(n) == "escalate" && n["config"]["reason_code"] == "low_confidence").map(|n| id(n).to_string());
                let Some(esc) = esc else { return deny("no_error_exit", "the flow has no low_confidence escalate node for the exhausted attempts of the new question") };
                json!({"id": nid, "type": "collect", "config": {"slot": slot, "prompt_ref": tref, "max_attempts": 2}, "next": {"ok": to, "max_attempts": esc}})
            }
            _ => json!({"id": nid, "type": "respond", "config": {"template_ref": tref}, "next": {"next": to}}),
        };
        let n = new_nodes.iter_mut().find(|n| id(n) == from).ok_or(Denied { code: "position_unknown", why: from.clone() })?;
        n["next"][label.as_str()] = json!(nid);
        new_nodes.push(node);
        templates.push(json!({"kind": "template", "content": {"id": tid, "version": "1.0.0", "locales": {"es": preset.es, "pt": preset.pt}},
                              "docs": {"description": format!("[improvement-engine] {op} text for {flow_id}"), "rationale": "reviewed es/pt text of an additive flow edit", "changelog": "new template"}}));
        node_id = Some(nid);
    }

    let mut f = flow.clone();
    f["nodes"] = Value::Array(new_nodes);
    f["version"] = json!(new_version);
    check_invariants(flow, &f)?;

    let mut a = agent.clone();
    // The drafted agent pins its entry flow: an EXACT registry pin must follow the new flow version (REG-PIN otherwise); a major pin still resolves.
    if ref_id(&a["entry_flow"]) == flow_id && a["entry_flow"].is_object() {
        a["entry_flow"]["spec"] = f["version"].clone();
    }
    a["version"] = json!(bump(agent["version"].as_str().unwrap_or(""), false));
    let mut facts = edit_facts(flow, &f, op, position, item["native_evidence"] == true, templates.len());
    facts["discriminator"] = item["discriminator"].clone();
    facts["chain"] = item["chain"].clone();
    let docs = |d: &str| json!({"description": d, "rationale": format!("additive flow edit {op}: no node is removed, protected nodes and their edges are unchanged"), "changelog": d});
    let mut changes = vec![
        json!({"kind": "flow", "content": f, "docs": docs(&format!("[improvement-engine] {op} at {position}: additive, pass-through, protected nodes untouched"))}),
        json!({"kind": "agent", "content": a, "docs": docs("[improvement-engine] entry flow pin follows the edited flow version")}),
    ];
    changes.extend(templates);
    Ok(Edit { changes, facts, node_id })
}

fn failure_labels(kind: &str) -> &'static [&'static str] {
    match kind {
        "decide" => &["low_confidence"],
        "collect" => &["max_attempts"],
        "tool" => &["error", "timeout", "denied"],
        "tool_write" => &["denied"],
        "confirm" => &["max_attempts"],
        "verify" => &["failed"],
        "agent" | "suggest" => &["gave_up"],
        _ => &[],
    }
}

/// G0-06 on one start node: the chain of plain responds ends in a collect, an escalation or a safe end.
fn safe_start(ns: &[Value], start: &str) -> bool {
    let mut cur = start.to_string();
    let mut seen = BTreeSet::new();
    loop {
        let Some(n) = get(ns, &cur) else { return true };
        match ty(n) {
            "collect" | "escalate" => return true,
            "end" => return ["abstained", "clarify_exhausted"].contains(&n["config"]["outcome"].as_str().unwrap_or("")),
            "respond" => {
                if n["config"]["claims"].as_array().is_some_and(|c| !c.is_empty()) {
                    return false;
                }
                if !seen.insert(cur.clone()) {
                    return true;
                }
                match n["next"]["next"].as_str() {
                    Some(t) => cur = t.to_string(),
                    None => return true,
                }
            }
            _ => return false,
        }
    }
}

fn safe_failures(ns: &[Value]) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for n in ns {
        for l in failure_labels(kind_of(n)) {
            if let Some(t) = n["next"][*l].as_str()
                && safe_start(ns, t)
            {
                out.insert((id(n).to_string(), l.to_string()));
            }
        }
    }
    out
}

/// Follows new (`eng_`) pass-through nodes to the first base node.
fn contract(nm: &BTreeMap<String, Value>, new_ids: &BTreeSet<String>, start: &str) -> Result<String, Denied> {
    let mut cur = start.to_string();
    for _ in 0..8 {
        if !new_ids.contains(&cur) {
            return Ok(cur);
        }
        let n = &nm[&cur];
        let pass = if ty(n) == "collect" { "ok" } else { "next" };
        let Some(t) = n["next"][pass].as_str() else { return deny("invariant_broken", format!("new node {cur} has no continuation")) };
        cur = t.to_string();
    }
    deny("invariant_broken", "a chain of new nodes does not end")
}

/// The deterministic recompute: `new` is the pass-through extension of `base` and touches no protected node or edge.
pub fn check_invariants(base: &Value, new: &Value) -> Result<(), Denied> {
    let broken = |why: String| -> Result<(), Denied> { deny("invariant_broken", why) };
    let (bn, nn) = (nodes(base), nodes(new));
    let nm: BTreeMap<String, Value> = nn.iter().map(|n| (id(n).to_string(), n.clone())).collect();
    let bm: BTreeSet<String> = bn.iter().map(|n| id(n).to_string()).collect();
    let new_ids: BTreeSet<String> = nm.keys().filter(|k| !bm.contains(*k)).cloned().collect();
    if bn.first().map(id) != nn.first().map(id) {
        return broken("the entry node changed".into());
    }
    if base["id"] != new["id"] || base["priority"] != new["priority"] {
        return broken("flow id or priority changed".into());
    }
    for b in &bn {
        let Some(n) = nm.get(id(b)) else { return broken(format!("node {} was removed", id(b))) };
        if ty(b) != ty(n) {
            return broken(format!("node {} changed type", id(b)));
        }
        if PROTECTED.contains(&kind_of(b)) {
            if b != n {
                return broken(format!("protected node {} ({}) changed", id(b), kind_of(b)));
            }
            continue;
        }
        let (mut bc, mut nc) = (b["config"].clone(), n["config"].clone());
        if ty(b) == "collect" && bc.get("validator").is_none_or(Value::is_null) {
            if let Some(o) = nc.as_object_mut() {
                o.remove("validator");
            }
            if let Some(o) = bc.as_object_mut() {
                o.remove("validator");
            }
        }
        if bc != nc {
            return broken(format!("the config of node {} changed", id(b)));
        }
        let (bx, nx): (BTreeMap<String, String>, BTreeMap<String, String>) = (next_of(b).into_iter().collect(), next_of(n).into_iter().collect());
        if bx.keys().ne(nx.keys()) {
            return broken(format!("the outputs of node {} changed", id(b)));
        }
        for (label, t) in &bx {
            if contract(&nm, &new_ids, &nx[label])? != *t {
                return broken(format!("{}.{label} no longer reaches {t}", id(b)));
            }
        }
    }
    for i in &new_ids {
        let n = &nm[i];
        if !i.starts_with(ENG) {
            return broken(format!("new node {i} lacks the engine prefix"));
        }
        let keys: BTreeSet<&str> = n["config"].as_object().into_iter().flat_map(|o| o.keys().map(String::as_str)).collect();
        match ty(n) {
            "respond" => {
                if keys != BTreeSet::from(["template_ref"]) || next_of(n).len() != 1 {
                    return broken(format!("new respond {i} is not a plain template respond"));
                }
            }
            "collect" => {
                let allowed = BTreeSet::from(["slot", "prompt_ref", "max_attempts"]);
                if !keys.is_subset(&allowed) || n["config"]["max_attempts"].as_u64().is_none_or(|m| m == 0 || m > 3) {
                    return broken(format!("new collect {i} has an unexpected config"));
                }
                let esc = n["next"]["max_attempts"].as_str().unwrap_or("");
                if bm.contains(esc) && get(&bn, esc).is_some_and(|e| ty(e) == "escalate") {
                    continue;
                }
                return broken(format!("new collect {i}: exhausted attempts must go to an existing escalate node"));
            }
            other => return broken(format!("new node {i} has the type {other}, not allowed")),
        }
    }
    let reach = reachable(&nn);
    if let Some(n) = nn.iter().find(|n| !reach.contains(id(n))) {
        return broken(format!("node {} is unreachable", id(n)));
    }
    if nn.len() > NODE_CAP {
        return broken(format!("more than {NODE_CAP} nodes"));
    }
    let after = safe_failures(&nn);
    if let Some((n, l)) = safe_failures(&bn).into_iter().find(|e| !after.contains(e)) {
        return broken(format!("the failure branch {n}.{l} was a safe exit and no longer is"));
    }
    Ok(())
}

/// Structured facts of an edit, recomputed from the two graphs (the independent review reads these, not the model's words).
pub fn edit_facts(base: &Value, new: &Value, op: &str, position: &str, native_evidence: bool, templates: usize) -> Value {
    let (bn, nn) = (nodes(base), nodes(new));
    let bm: BTreeSet<String> = bn.iter().map(|n| id(n).to_string()).collect();
    let added: Vec<&Value> = nn.iter().filter(|n| !bm.contains(id(n))).collect();
    let (from, to) = match (op, position.rsplit_once('.')) {
        ("add_validator", _) | (_, None) => (position.to_string(), String::new()),
        (_, Some((f, l))) => (f.to_string(), get(&bn, f).and_then(|n| n["next"][l].as_str()).unwrap_or("").to_string()),
    };
    let protected_unchanged = bn.iter().filter(|b| PROTECTED.contains(&kind_of(b))).all(|b| get(&nn, id(b)) == Some(b));
    let exits: Vec<Value> = added.iter().flat_map(|n| next_of(n).into_iter().filter(|(l, _)| !["next", "ok"].contains(&l.as_str())).map(|(l, t)| json!({"node": id(n), "label": l, "to_type": get(&nn, &t).map_or("", |x| ty(x))})).collect::<Vec<_>>()).collect();
    let esc = |ns: &[Value]| -> usize { let r = reachable(ns); ns.iter().filter(|n| ty(n) == "escalate" && r.contains(id(n))).count() };
    json!({"op": op, "flow": base["id"], "position": position, "from": {"id": from, "type": get(&bn, &from).map_or("", |n| kind_of(n))},
           "to": if to.is_empty() { Value::Null } else { json!({"id": to, "type": get(&bn, &to).map_or("", |n| ty(n))}) },
           "new_nodes": added.iter().map(|n| json!({"id": id(n), "type": ty(n)})).collect::<Vec<_>>(), "exits_of_new_nodes": exits,
           "validator_added": op == "add_validator", "templates_added": templates, "protected_unchanged": protected_unchanged,
           "pass_through": check_invariants(base, new).is_ok(), "escalations_reachable": {"before": esc(&bn), "after": esc(&nn)}, "native_evidence": native_evidence})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn flow(name: &str) -> Value {
        Catalog::bundled().flow(name).expect("bundled flow").clone()
    }
    fn agent(name: &str) -> Value {
        Catalog::bundled().agent(name).expect("bundled agent").clone()
    }
    fn p_text(id: &str) -> Preset {
        Preset { id: id.into(), es: "Estoy revisando tu solicitud ahora.".into(), pt: "Estou analisando sua solicitacao agora.".into(), slot: None, validator: None }
    }
    fn p_ask() -> Preset {
        Preset { id: "ask_fecha".into(), es: "Para ayudarte mejor, recuerdas la fecha aproximada?".into(), pt: "Para ajudar melhor, voce lembra a data aproximada?".into(), slot: Some("fecha_aproximada".into()), validator: None }
    }
    fn p_val() -> Preset {
        Preset { id: "radicado_token".into(), es: String::new(), pt: String::new(), slot: None, validator: Some(json!({"kind": "extract", "value": "([a-z]{2,5}-[a-z0-9-]*[0-9]+|[0-9]{6,})"})) }
    }
    fn free(_: &str) -> bool {
        false
    }
    fn code(r: Result<Edit, Denied>) -> &'static str {
        r.expect_err("must be denied").code
    }
    fn ids(m: &[Value]) -> Vec<String> {
        m.iter().map(|x| x["position_id"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn menus_are_derived_from_the_real_graph_and_never_offer_a_protected_edge() {
        let f = flow("disputa-cargo");
        let ack = ids(&menu("insert_ack", &f));
        for want in ["pedir_cargo.ok", "buscar_tx.ok", "elegir.ok", "a_usd.ok"] {
            assert!(ack.contains(&want.to_string()), "{want} missing in {ack:?}");
        }
        for frozen in ["coincide.unica", "umbral.true", "umbral.false", "confirmar.true", "radicar.ok", "verificar.verified", "aclarar.next", "responder_ok.next", "esc_monto.x"] {
            assert!(!ack.contains(&frozen.to_string()), "{frozen} must not be on the menu");
        }
        let notice = ids(&menu("insert_notice", &f));
        assert!(notice.contains(&"buscar_tx.error".to_string()) && notice.contains(&"pedir_cargo.max_attempts".to_string()));
        assert!(!notice.contains(&"radicar.denied".to_string()), "a write tool failure is not on the menu");
        assert!(!notice.contains(&"verificar.failed".to_string()) && !notice.contains(&"umbral.true".to_string()));
        let val = ids(&menu("add_validator", &f));
        assert_eq!(val, vec!["pedir_cargo"]);
        // native evidence only where a scripted run reaches the position deterministically
        let ask = menu("insert_ask", &flow("consulta-pqr"));
        let by = |p: &str| ask.iter().find(|m| m["position_id"] == p).unwrap().clone();
        assert_eq!(by("pedir_radicado.ok")["native_evidence"], true);
        assert_eq!(by("consultar.ok")["native_evidence"], true);
        assert_eq!(by("consultar.ok")["chain"][1]["tool"], "obtener_pqr");
        assert_eq!(by("pedir_radicado.ok")["discriminator"], "tool_not_called", "the base calls obtener_pqr right after, the candidate asks first");
        assert_eq!(by("consultar.ok")["discriminator"], "run_not_closed", "the base answers and closes, the candidate waits for the answer");
        assert_eq!(menu("insert_ask", &flow("disputa-cargo")).iter().find(|m| m["position_id"] == "pedir_cargo.ok").unwrap()["discriminator"], "tool_not_called");
        assert_eq!(menu("add_validator", &flow("consulta-pqr"))[0]["discriminator"], "tool_not_called");
        let d = menu("insert_ask", &f);
        assert_eq!(d.iter().find(|m| m["position_id"] == "a_usd.ok").unwrap()["native_evidence"], false, "behind a decide the path is not deterministic");
        assert!(menu("insert_ack", &f).iter().all(|m| m["native_evidence"] == false), "a plain respond has no native evidence");
    }

    #[test]
    fn insert_ask_adds_a_collect_with_an_existing_exit_a_template_and_follows_the_pins() {
        let mut a = agent("consultas");
        a["entry_flow"] = json!({"id": "consulta-pqr", "spec": "1.0.0"});
        let e = compile_flow_edit(&flow("consulta-pqr"), &a, "insert_ask", "pedir_radicado.ok", &p_ask(), &free).unwrap();
        let f = &e.changes[0]["content"];
        assert_eq!(f["version"], "1.1.0");
        let ns = f["nodes"].as_array().unwrap();
        let n = ns.iter().find(|n| n["id"] == "eng_ask_pedir_radicado_ok").unwrap();
        assert_eq!((n["type"].as_str(), n["next"]["ok"].as_str(), n["next"]["max_attempts"].as_str()), (Some("collect"), Some("consultar"), Some("esc_sin_datos")));
        assert_eq!(ns.iter().find(|x| x["id"] == "pedir_radicado").unwrap()["next"]["ok"], "eng_ask_pedir_radicado_ok");
        assert_eq!(n["config"]["slot"], "fecha_aproximada");
        assert_eq!(e.changes[1]["content"]["version"], "1.0.1");
        assert_eq!(e.changes[1]["content"]["entry_flow"]["spec"], "1.1.0", "REG-PIN: the exact pin follows the new flow version");
        let t = &e.changes[2];
        assert_eq!((t["kind"].as_str(), t["content"]["version"].as_str()), (Some("template"), Some("1.0.0")));
        assert!(t["content"]["locales"]["es"].as_str().unwrap().contains("fecha"));
        assert_eq!(e.facts["pass_through"], true);
        assert_eq!(e.facts["protected_unchanged"], true);
        assert_eq!(e.facts["escalations_reachable"], json!({"before": 2, "after": 2}));
        // a major pin (fixture style) stays as it is
        let e2 = compile_flow_edit(&flow("consulta-pqr"), &agent("consultas"), "insert_ask", "pedir_radicado.ok", &p_ask(), &free).unwrap();
        assert_eq!(e2.changes[1]["content"]["entry_flow"], "consulta-pqr@1");
    }

    #[test]
    fn insert_notice_sits_between_the_failing_edge_and_its_existing_escalation() {
        let e = compile_flow_edit(&flow("consulta-pqr"), &agent("consultas"), "insert_notice", "consultar.error", &p_text("n1"), &free).unwrap();
        let ns = e.changes[0]["content"]["nodes"].as_array().unwrap().clone();
        let n = ns.iter().find(|n| n["id"] == "eng_notice_consultar_error").unwrap();
        assert_eq!((n["type"].as_str(), n["next"]["next"].as_str()), (Some("respond"), Some("esc_tool")));
        let c = ns.iter().find(|n| n["id"] == "consultar").unwrap();
        assert_eq!((c["next"]["error"].as_str(), c["next"]["timeout"].as_str(), c["next"]["denied"].as_str()), (Some("eng_notice_consultar_error"), Some("esc_tool"), Some("esc_tool")));
        assert_eq!(ns.iter().find(|n| n["id"] == "esc_tool").unwrap(), &flow("consulta-pqr")["nodes"][5], "the escalation is untouched");
    }

    #[test]
    fn insert_ack_is_a_pass_through_respond_in_the_style_of_the_flow() {
        let e = compile_flow_edit(&flow("consulta-pqr"), &agent("consultas"), "insert_ack", "pedir_radicado.ok", &p_text("a1"), &free).unwrap();
        let ns = e.changes[0]["content"]["nodes"].as_array().unwrap().clone();
        let n = ns.iter().find(|n| n["id"] == "eng_ack_pedir_radicado_ok").unwrap();
        assert_eq!((n["next"]["next"].as_str(), n["config"]["template_ref"].as_str()), (Some("consultar"), Some("t/eng_ack_pedir_radicado_ok_consulta_pqr")));
        // registry style: object refs with an exact spec
        let mut f = flow("consulta-pqr");
        f["nodes"][0]["config"]["prompt_ref"] = json!({"id": "t/pedir_radicado", "spec": "1.0.0"});
        let e2 = compile_flow_edit(&f, &agent("consultas"), "insert_ack", "pedir_radicado.ok", &p_text("a1"), &free).unwrap();
        let n2 = e2.changes[0]["content"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "eng_ack_pedir_radicado_ok").unwrap().clone();
        assert_eq!(n2["config"]["template_ref"], json!({"id": "t/eng_ack_pedir_radicado_ok_consulta_pqr", "spec": "1.0.0"}));
    }

    #[test]
    fn add_validator_changes_only_the_validator_of_a_collect_that_has_none() {
        let e = compile_flow_edit(&flow("consulta-pqr"), &agent("consultas"), "add_validator", "pedir_radicado", &p_val(), &free).unwrap();
        assert_eq!(e.changes.len(), 2, "no template, no new node");
        let ns = e.changes[0]["content"]["nodes"].as_array().unwrap().clone();
        assert_eq!(ns.len(), flow("consulta-pqr")["nodes"].as_array().unwrap().len());
        assert_eq!(ns[0]["config"]["validator"]["kind"], "extract");
        assert_eq!(ns[0]["config"]["max_attempts"], 2);
        assert_eq!(e.facts["validator_added"], true);
        // a collect that has a validator is not edited again
        assert_eq!(code(compile_flow_edit(&e.changes[0]["content"], &agent("consultas"), "add_validator", "pedir_radicado", &p_val(), &free)), "validator_exists");
        assert_eq!(code(compile_flow_edit(&flow("consulta-pqr"), &agent("consultas"), "add_validator", "consultar", &p_val(), &free)), "node_protected");
        assert_eq!(code(compile_flow_edit(&flow("consulta-pqr"), &agent("consultas"), "add_validator", "umbral", &p_val(), &free)), "position_unknown");
        let bad = |v: Value| code(compile_flow_edit(&flow("consulta-pqr"), &agent("consultas"), "add_validator", "pedir_radicado", &Preset { validator: Some(v), ..p_val() }, &free));
        assert_eq!(bad(json!({"kind": "decide", "value": "x"})), "validator_invalid");
        assert_eq!(bad(json!({"kind": "extract", "value": "a|b"})), "validator_invalid", "extract needs one capture group");
        assert_eq!(bad(json!({"kind": "regex", "value": "((a+)+)b"})), "validator_invalid", "nested quantifiers");
        assert_eq!(bad(json!({"kind": "regex", "value": "(a"})), "validator_invalid");
        assert_eq!(bad(json!({"kind": "enum", "value": []})), "validator_invalid");
        assert!(check_validator(&json!({"kind": "extract", "value": "(?i)\\b(pqr-[a-z0-9]+)\\b"})).is_ok());
    }

    #[test]
    fn protected_nodes_and_their_edges_are_refused() {
        let f = flow("disputa-cargo");
        let a = agent("disputas");
        for (pos, want) in [
            ("umbral.true", "edge_frozen"), ("umbral.false", "edge_frozen"), ("coincide.unica", "edge_frozen"), ("confirmar.true", "edge_frozen"), ("confirmar.unclear", "edge_frozen"),
            ("radicar.ok", "edge_frozen"), ("verificar.verified", "edge_frozen"), ("verificar.failed", "edge_frozen"), ("fin.next", "position_unknown"), ("esc_monto.x", "position_unknown"),
            ("aclarar.next", "position_not_allowed"), ("responder_ok.next", "position_not_allowed"),
        ] {
            for op in ["insert_ack", "insert_ask", "insert_notice"] {
                assert_eq!(code(compile_flow_edit(&f, &a, op, pos, &p_ask(), &free)), want, "{op} at {pos}");
            }
        }
        // a notice only precedes a plain failure exit, never the policy escalation or a failed write
        assert_eq!(code(compile_flow_edit(&f, &a, "insert_notice", "radicar.denied", &p_text("n"), &free)), "edge_frozen");
        assert_eq!(code(compile_flow_edit(&f, &a, "insert_notice", "buscar_tx.ok", &p_text("n"), &free)), "position_not_allowed");
        // a protected node tampered with by any means is caught by the recompute
        let mut t = f.clone();
        let nodes = t["nodes"].as_array_mut().unwrap();
        let rule = nodes.iter_mut().find(|n| n["id"] == "umbral").unwrap();
        rule["next"]["true"] = json!("confirmar");
        assert_eq!(check_invariants(&f, &t).unwrap_err().code, "invariant_broken");
        let mut t = f.clone();
        t["nodes"].as_array_mut().unwrap().retain(|n| n["id"] != "esc_monto");
        t["nodes"].as_array_mut().unwrap().iter_mut().find(|n| n["id"] == "umbral").unwrap()["next"]["true"] = json!("esc_tool");
        assert_eq!(check_invariants(&f, &t).unwrap_err().code, "invariant_broken");
    }

    #[test]
    fn unknown_positions_ops_and_invalid_graphs_are_refused() {
        let f = flow("consulta-pqr");
        let a = agent("consultas");
        assert_eq!(code(compile_flow_edit(&f, &a, "insert_ask", "nope.ok", &p_ask(), &free)), "position_unknown");
        assert_eq!(code(compile_flow_edit(&f, &a, "insert_ask", "pedir_radicado.zzz", &p_ask(), &free)), "position_unknown");
        assert_eq!(code(compile_flow_edit(&f, &a, "insert_ask", "pedir_radicado", &p_ask(), &free)), "position_unknown");
        assert_eq!(code(compile_flow_edit(&f, &a, "delete_node", "consultar", &p_ask(), &free)), "op_unknown");
        assert_eq!(code(compile_flow_edit(&flow("construir"), &agent("constructor-chat"), "insert_ack", "pedir_agente.ok", &p_text("a"), &free)), "flow_unsupported");
        let mut dangling = f.clone();
        dangling["nodes"][1]["next"]["ok"] = json!("ghost");
        assert_eq!(code(compile_flow_edit(&dangling, &a, "insert_ack", "pedir_radicado.ok", &p_text("a"), &free)), "invalid_graph");
        let mut orphan = f.clone();
        orphan["nodes"].as_array_mut().unwrap().push(json!({"id": "huerfano", "type": "end", "config": {"outcome": "abstained"}}));
        assert_eq!(code(compile_flow_edit(&orphan, &a, "insert_ack", "pedir_radicado.ok", &p_text("a"), &free)), "invalid_graph");
        let mut dup = f.clone();
        dup["nodes"].as_array_mut().unwrap().push(f["nodes"][0].clone());
        assert_eq!(validate_graph(&dup).unwrap_err().code, "invalid_graph");
        let mut missing_label = f.clone();
        missing_label["nodes"][0]["next"].as_object_mut().unwrap().remove("max_attempts");
        assert_eq!(validate_graph(&missing_label).unwrap_err().code, "invalid_graph");
        // an ask needs an existing low_confidence exit for its exhausted attempts
        let mut no_exit = f.clone();
        no_exit["nodes"][4]["config"]["reason_code"] = json!("customer_request");
        no_exit["nodes"][0]["next"]["max_attempts"] = json!("esc_sin_datos");
        assert_eq!(code(compile_flow_edit(&no_exit, &a, "insert_ask", "pedir_radicado.ok", &p_ask(), &free)), "no_error_exit");
    }

    #[test]
    fn presets_texts_slots_ids_and_limits_are_checked() {
        let f = flow("consulta-pqr");
        let a = agent("consultas");
        let with = |p: Preset| code(compile_flow_edit(&f, &a, "insert_ask", "pedir_radicado.ok", &p, &free));
        assert_eq!(with(Preset { es: "Recuerdas el numero 123456 del cargo".into(), ..p_ask() }), "text_not_clean");
        assert_eq!(with(Preset { es: "Hola {{ slots.x }} amigo mio".into(), ..p_ask() }), "text_not_clean");
        assert_eq!(with(Preset { pt: p_ask().es, ..p_ask() }), "translation_copy");
        assert_eq!(with(Preset { slot: Some("radicado".into()), ..p_ask() }), "slot_taken");
        assert_eq!(with(Preset { slot: Some("Bad Slot".into()), ..p_ask() }), "preset_invalid");
        assert_eq!(code(compile_flow_edit(&f, &a, "insert_ask", "pedir_radicado.ok", &p_ask(), &|t| t.starts_with("t/eng_ask"))), "template_taken");
        // the same edit cannot be applied twice, and the engine adds at most MAX_ENGINE_NODES nodes to one flow
        let once = compile_flow_edit(&f, &a, "insert_ack", "pedir_radicado.ok", &p_text("a"), &free).unwrap();
        let g = once.changes[0]["content"].clone();
        assert_eq!(code(compile_flow_edit(&g, &a, "insert_ack", "pedir_radicado.ok", &p_text("a"), &free)), "already_applied", "the same edit is not applied twice");
        let mut cur = f.clone();
        for (i, pos) in ["consultar.ok", "consultar.error", "pedir_radicado.max_attempts"].iter().enumerate() {
            let op = if i == 0 { "insert_ack" } else { "insert_notice" };
            cur = compile_flow_edit(&cur, &a, op, pos, &p_text("n"), &free).unwrap().changes[0]["content"].clone();
        }
        assert_eq!(code(compile_flow_edit(&cur, &a, "insert_ack", "pedir_radicado.ok", &p_text("a"), &free)), "edit_limit");
        // a validator is not an added node: it is allowed after the limit
        assert!(compile_flow_edit(&cur, &a, "add_validator", "pedir_radicado", &p_val(), &free).is_ok());
    }

    #[test]
    fn every_menu_position_of_every_real_flow_compiles_into_a_valid_pass_through() {
        for name in ["consulta-pqr", "disputa-cargo", "recepcion"] {
            let f = flow(name);
            let a = agent(match name { "consulta-pqr" => "consultas", "disputa-cargo" => "disputas", _ => "recepcion" });
            for op in OPS {
                for m in menu(op, &f) {
                    let pos = m["position_id"].as_str().unwrap();
                    let preset = match op {
                        "add_validator" => p_val(),
                        "insert_ask" => p_ask(),
                        _ => p_text("x"),
                    };
                    let e = compile_flow_edit(&f, &a, op, pos, &preset, &free);
                    match e {
                        Ok(e) => {
                            check_invariants(&f, &e.changes[0]["content"]).unwrap();
                            assert_eq!(e.facts["pass_through"], true, "{name} {op} {pos}");
                        }
                        // the only legitimate refusal over a real flow: an ask in a flow with no low_confidence exit
                        Err(d) => assert!(d.code == "no_error_exit", "{name} {op} {pos}: {} {}", d.code, d.why),
                    }
                }
            }
        }
    }
}
