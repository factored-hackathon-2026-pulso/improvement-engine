//! The byte-exact compiler of the Builder's proposal.
//!
//! `compile` applies an ANCHORED PATCH to the exact base text of a real agent-core artifact (the Builder names anchor ids from the
//! menu the engine computed; it never supplies a whole text), or derives a new specialist agent by closure copy of the donor
//! `consultas`. Everything the model wrote is re-checked here: anchors resolve and are unique, every locale is patched, the pt
//! replacement is a real parallel (not a copy of the es one), placeholders are whitelisted, no safety marker of the base is lost,
//! the edit stays within budget, no text carries PII-like patterns. A failure is a `Denied` with a closed reason code.
use crate::catalog::{Artifact, Catalog, anchors, protected_markers};
use crate::clean_text;
use crate::finding::{Finding, round2};
use crate::mapping::{Row, Target};
use crate::roles::{LOCALES, Opportunity, edit_budget};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    pub code: &'static str,
    pub why: String,
}

fn deny<T>(code: &'static str, why: impl Into<String>) -> Result<T, Denied> {
    Err(Denied { code, why: why.into() })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    /// `patch | new_agent | no_change`
    pub kind: String,
    pub target_ref: String,
    /// The agent the registry proposal is opened for: the agent whose behaviour the target changes, or the slug of a new agent.
    pub agent_id: String,
    /// Agent-core draft changes: `{kind, content, docs}` (empty for `no_change`).
    pub changes: Vec<Value>,
    /// Human-readable diff: one entry per patch (anchor id, exact anchor text, op, replacement) or one per new entity.
    pub diff: Vec<Value>,
    /// Digest of the base the patch was applied to (a changed live base must be detected by comparing it).
    pub base_digest: String,
    /// Entities predicted to move to the next patch version (compare with the Core's `auto_bumped`).
    pub cascade: Vec<String>,
    /// Characters removed plus characters written (rubric R3 / R12).
    pub edit_chars: usize,
    pub edit_budget: usize,
    /// Release-level items only a human admin can set (new agent) or other human-owned follow-ups.
    pub human_items: Vec<String>,
    pub expected_effect: Value,
    pub rationale: String,
    pub uncertainty: String,
}

impl Compiled {
    pub fn to_json(&self) -> Value {
        json!({"kind": self.kind, "target_ref": self.target_ref, "agent_id": self.agent_id, "changes": self.changes, "diff": self.diff, "base_digest": self.base_digest, "cascade": self.cascade,
               "edit_chars": self.edit_chars, "edit_budget": self.edit_budget, "human_items": self.human_items, "expected_effect": self.expected_effect,
               "rationale": self.rationale, "uncertainty": self.uncertainty,
               "rollback": {"how": "revert to the base release; staging only, no release_settings change", "base_digest": self.base_digest}})
    }
}

pub fn bump_patch(v: &str) -> String {
    let p: Vec<u64> = v.split('.').filter_map(|x| x.parse().ok()).collect();
    if p.len() == 3 { format!("{}.{}.{}", p[0], p[1], p[2] + 1) } else { v.to_string() }
}

/// Every `{{ x }}` of a text; an unbalanced `{{` or a stray `}}` is an error.
fn placeholders(t: &str) -> Result<Vec<String>, String> {
    let mut out = vec![];
    let mut rest = t;
    while let Some(i) = rest.find("{{") {
        let after = &rest[i + 2..];
        let j = after.find("}}").ok_or("unclosed {{")?;
        if after[..j].contains("{{") {
            return Err("nested {{".into());
        }
        out.push(after[..j].trim().to_string());
        rest = &after[j + 2..];
    }
    if rest.contains("}}") {
        return Err("stray }}".into());
    }
    Ok(out)
}

fn expected_effect(f: &Finding, row: &Row, direction: &str) -> Value {
    json!({"metric_id": f.metric_token(), "population": f.dims, "direction": direction, "current_cell_rate": round2(f.discovery.rate),
           "reference_rate": round2(f.discovery.baseline_rate), "min_detectable_gap": round2(f.discovery.diff / 2.0),
           "success_if": "the cell rate falls by at least min_detectable_gap toward the reference rate over a new window with the same k-anonymity",
           "guardrail": row.guardrail, "link_grade": row.link_grade, "evidence_ref": f.evidence_ref()})
}

fn docs(f: &Finding, row: &Row, opp: &Opportunity, rationale: &str) -> Value {
    let text = format!("[improvement-engine] {} {} {}; finding {} evidence {} link {}; hypothesis: {}; rationale: {}", opp.target_ref, opp.mechanism_class, row.id, f.id, f.evidence_ref(), row.link_grade, opp.hypothesis, rationale);
    json!({"description": text.chars().take(4000).collect::<String>(), "rationale": rationale.chars().take(4000).collect::<String>()})
}

pub fn compile(catalog: &Catalog, f: &Finding, row: &Row, opp: &Opportunity, proposal: &Value) -> Result<Compiled, Denied> {
    let kind = proposal["kind"].as_str().unwrap_or("");
    let rationale = proposal["rationale"].as_str().unwrap_or("").to_string();
    let uncertainty = proposal["uncertainty"].as_str().unwrap_or("").to_string();
    let direction = proposal["expected_direction"].as_str().unwrap_or("decrease");
    let target = row.target(&opp.target_ref).ok_or_else(|| Denied { code: "target_mismatch", why: "the opportunity target is not in the mapping row".into() })?;
    if kind == "no_change" {
        return Ok(Compiled { kind: "no_change".into(), target_ref: opp.target_ref.clone(), agent_id: target.agent.to_string(), changes: vec![], diff: vec![], base_digest: String::new(), cascade: vec![], edit_chars: 0, edit_budget: 0,
                             human_items: vec![], expected_effect: Value::Null, rationale, uncertainty });
    }
    if proposal.get("target_ref").and_then(Value::as_str).is_some_and(|t| t != opp.target_ref) {
        return deny("target_mismatch", "the proposal targets another artifact than the verified opportunity");
    }
    if kind != target.kind {
        return deny("kind_mismatch", format!("the target {} takes kind {}, the proposal is {kind}", opp.target_ref, target.kind));
    }
    if f.direction != "up" || direction != "decrease" {
        return deny("direction_mismatch", "every cells metric is higher-is-worse: the finding must be up and the expected direction decrease");
    }
    let docs = docs(f, row, opp, &rationale);
    let effect = expected_effect(f, row, direction);
    match kind {
        "patch" => compile_patch(catalog, target, proposal, docs, effect, rationale, uncertainty),
        "new_agent" => compile_new_agent(catalog, target, proposal, docs, effect, rationale, uncertainty),
        other => deny("kind_mismatch", format!("unknown kind {other:?}")),
    }
}

fn compile_patch(catalog: &Catalog, target: &Target, p: &Value, docs: Value, effect: Value, rationale: String, uncertainty: String) -> Result<Compiled, Denied> {
    let art: &Artifact = catalog.get(&target.target_ref).ok_or_else(|| Denied { code: "target_mismatch", why: format!("{} is not in the baseline catalogue", target.target_ref) })?;
    let patches = p["patches"].as_array().ok_or_else(|| Denied { code: "anchor_unknown", why: "no patches".into() })?;
    let mut by_locale: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    for (i, x) in patches.iter().enumerate() {
        let locale = x["locale"].as_str().unwrap_or("");
        if !art.locales.contains_key(locale) || !LOCALES.contains(&locale) {
            return deny("locale_parity", format!("patch {i} names locale {locale:?}, which the artifact does not have"));
        }
        let op = x["op"].as_str().unwrap_or("");
        if !["replace", "insert_after"].contains(&op) {
            return deny("op_not_allowed", format!("patch {i}: op {op:?}"));
        }
        let (aid, rep) = (x["anchor_id"].as_str().unwrap_or(""), x["replacement"].as_str().unwrap_or(""));
        clean_text(rep, 400).map_err(|e| Denied { code: "text_not_clean", why: format!("patch {i}: {e}") })?;
        if art.kind == "prompt" && (rep.contains("{{") || rep.contains("}}")) {
            return deny("placeholder_not_allowed", format!("patch {i}: a prompt takes no placeholders"));
        }
        for ph in placeholders(rep).map_err(|e| Denied { code: "placeholder_not_allowed", why: format!("patch {i}: {e}") })? {
            if !target.placeholders.contains(&ph.as_str()) {
                return deny("placeholder_not_allowed", format!("patch {i}: placeholder {ph:?} is not whitelisted"));
            }
        }
        by_locale.entry(locale.to_string()).or_default().push((aid.to_string(), op.to_string(), rep.to_string()));
    }
    let mut new_locales = art.locales.clone();
    let (mut diff, mut edit_chars) = (vec![], 0usize);
    let mut reps: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for locale in art.locales.keys() {
        let list = by_locale.get(locale).ok_or_else(|| Denied { code: "locale_parity", why: format!("no patch for locale {locale}: every locale of the artifact must be patched") })?;
        if list.len() > 2 {
            return deny("edit_budget_exceeded", format!("more than 2 patches for locale {locale}"));
        }
        let base = &art.locales[locale];
        let menu = anchors(art, locale);
        let mut spans: Vec<(usize, usize, String, String, String)> = vec![];
        for (aid, op, rep) in list {
            let a = menu.iter().find(|a| &a.id == aid).ok_or_else(|| Denied { code: "anchor_unknown", why: format!("anchor {aid:?} is not on the menu of {locale} (protected or unknown)") })?;
            if base.matches(a.text.as_str()).count() != 1 {
                return deny("anchor_not_unique", format!("anchor {aid} does not occur exactly once"));
            }
            let at = base.find(a.text.as_str()).unwrap_or(0);
            spans.push((at, at + a.text.len(), op.clone(), rep.clone(), aid.clone()));
            edit_chars += rep.chars().count() + if op == "replace" { a.text.chars().count() } else { 0 };
            diff.push(json!({"locale": locale, "anchor_id": aid, "op": op, "anchor_text": a.text, "replacement": rep}));
            reps.entry(locale.clone()).or_default().push(rep.clone());
        }
        spans.sort_by_key(|s| s.0);
        if spans.windows(2).any(|w| w[0].1 > w[1].0) || spans.windows(2).any(|w| w[0].4 == w[1].4) {
            return deny("patch_overlap", format!("patches of locale {locale} overlap or repeat an anchor"));
        }
        let mut out = String::new();
        let mut cursor = 0;
        for (s, e, op, rep, _) in &spans {
            out.push_str(&base[cursor..*s]);
            if op == "replace" {
                out.push_str(rep);
            } else {
                out.push_str(&base[*s..*e]);
                out.push(' ');
                out.push_str(rep);
            }
            cursor = *e;
        }
        out.push_str(&base[cursor..]);
        for m in protected_markers(art, locale) {
            if base.contains(m) && !out.contains(m) {
                return deny("protected_clause_lost", format!("locale {locale}: the marker {m:?} of the base is gone"));
            }
        }
        new_locales.insert(locale.clone(), out);
    }
    let budget = edit_budget(art);
    if edit_chars > budget {
        return deny("edit_budget_exceeded", format!("{edit_chars} characters edited, budget {budget}"));
    }
    if let (Some(es), Some(pt)) = (reps.get("es"), reps.get("pt"))
        && es.iter().any(|e| pt.contains(e))
    {
        return deny("translation_copy", "a pt replacement equals an es replacement");
    }
    let mut content = json!({"id": art.id, "version": bump_patch(&art.version), "locales": new_locales});
    if let (Some(c), Some(x)) = (content.as_object_mut(), art.extra.as_object()) {
        c.extend(x.clone());
    }
    Ok(Compiled {
        kind: "patch".into(),
        target_ref: target.target_ref.clone(),
        agent_id: target.agent.to_string(),
        changes: vec![json!({"kind": art.kind, "content": content, "docs": docs})],
        diff,
        base_digest: art.digest(),
        cascade: catalog.cascade(art),
        edit_chars,
        edit_budget: budget,
        human_items: vec![],
        expected_effect: effect,
        rationale,
        uncertainty,
    })
}

const BROAD: [&str; 8] = ["any problem", "cualquier", "qualquer", "todo tipo", "todos los", "todos os", "everything", "anything"];

fn texts(p: &Value, key: &str, max: usize, min_len: usize) -> Result<String, Denied> {
    let t = p.get(key).and_then(Value::as_str).unwrap_or("");
    if t.chars().count() < min_len {
        return deny("routing_invalid", format!("{key} is missing or too short"));
    }
    clean_text(t, max).map_err(|e| Denied { code: "text_not_clean", why: format!("{key}: {e}") })?;
    Ok(t.to_string())
}

fn examples(p: &Value, key: &str) -> Result<Vec<String>, Denied> {
    let arr = p.get(key).and_then(Value::as_array).ok_or_else(|| Denied { code: "routing_invalid", why: format!("{key} is not a list") })?;
    if !(2..=4).contains(&arr.len()) {
        return deny("routing_invalid", format!("{key}: two to four examples"));
    }
    arr.iter()
        .map(|e| {
            let t = e.as_str().unwrap_or("");
            if t.chars().count() < 3 {
                return deny("routing_invalid", format!("{key}: an example is too short"));
            }
            clean_text(t, 100).map_err(|e| Denied { code: "text_not_clean", why: format!("{key}: {e}") })?;
            Ok(t.to_string())
        })
        .collect()
}

fn compile_new_agent(catalog: &Catalog, target: &Target, p: &Value, docs: Value, effect: Value, rationale: String, uncertainty: String) -> Result<Compiled, Denied> {
    let slug = p["agent_id"].as_str().unwrap_or("");
    if !target.slugs.contains(&slug) {
        return deny("slug_not_allowed", format!("agent_id {slug:?} is not one of the allowed slugs"));
    }
    if catalog.agent(slug).is_some() {
        return deny("slug_not_allowed", "an agent with that id already exists");
    }
    let donor = catalog.agent(&catalog.donor).ok_or_else(|| Denied { code: "target_mismatch", why: "the donor agent is not in the catalogue".into() })?;
    let routing = p.get("routing").unwrap_or(&Value::Null);
    let (sum_es, sum_pt) = (texts(routing, "summary_es", 240, 20)?, texts(routing, "summary_pt", 240, 20)?);
    let (ex_es, ex_pt) = (examples(routing, "examples_es")?, examples(routing, "examples_pt")?);
    if sum_es == sum_pt || ex_es.iter().any(|e| ex_pt.contains(e)) {
        return deny("translation_copy", "the pt routing text equals the es one");
    }
    let lower = format!("{sum_es} {sum_pt}").to_lowercase();
    if let Some(b) = BROAD.iter().find(|b| lower.contains(*b)) {
        return deny("routing_invalid", format!("the routing summary is broad ({b:?}) and could take traffic of the sibling agents"));
    }
    let intake = p.get("intake").unwrap_or(&Value::Null);
    let (ask_es, ask_pt, note_es, note_pt) = (texts(intake, "ask_es", 240, 8)?, texts(intake, "ask_pt", 240, 8)?, texts(intake, "notice_es", 240, 8)?, texts(intake, "notice_pt", 240, 8)?);
    if ask_es == ask_pt || note_es == note_pt {
        return deny("translation_copy", "the pt intake text equals the es one");
    }
    // Traffic-stealing guard against the sibling routing cards: no example may repeat a sibling example.
    for id in catalog.agent_ids() {
        for e in catalog.agent(&id).and_then(|a| a["routing"]["examples"].as_array()).into_iter().flatten().filter_map(Value::as_str) {
            if ex_es.iter().chain(&ex_pt).any(|x| x.eq_ignore_ascii_case(e)) {
                return deny("routing_invalid", format!("an example repeats one of the sibling agent {id}"));
            }
        }
    }
    let mut agent = donor.clone();
    let a = agent.as_object_mut().ok_or_else(|| Denied { code: "target_mismatch", why: "donor agent is not an object".into() })?;
    a.insert("id".into(), json!(slug));
    a.insert("version".into(), json!("1.0.0"));
    a.insert("entry_flow".into(), json!(format!("{slug}-intake@1")));
    a.insert("tools_allowed".into(), json!([]));
    a.insert("routing".into(), json!({"directory": donor["routing"]["directory"], "summary": format!("{sum_es} / {sum_pt}"), "examples": ex_es.iter().chain(&ex_pt).collect::<Vec<_>>()}));
    let (pedir, aviso) = (format!("t/{slug}_pedir"), format!("t/{slug}_aviso"));
    let flow = json!({"id": format!("{slug}-intake"), "version": "1.0.0", "priority": 50, "nodes": [
        {"id": "pedir_problema", "type": "collect", "config": {"slot": "problema", "prompt_ref": pedir, "max_attempts": 2}, "next": {"ok": "avisar", "max_attempts": "esc_sin_datos"}},
        {"id": "avisar", "type": "respond", "config": {"template_ref": aviso}, "next": {"next": "derivar"}},
        {"id": "derivar", "type": "escalate", "config": {"reason_code": "customer_request"}},
        {"id": "esc_sin_datos", "type": "escalate", "config": {"reason_code": "low_confidence"}}]});
    let tpl = |id: &str, es: &str, pt: &str| json!({"id": id, "version": "1.0.0", "locales": {"es": es, "pt": pt}});
    let changes = vec![
        json!({"kind": "agent", "content": agent, "docs": docs}),
        json!({"kind": "flow", "content": flow, "docs": {"description": format!("[improvement-engine] intake flow of the new agent {slug}")}}),
        json!({"kind": "template", "content": tpl(&pedir, &ask_es, &ask_pt), "docs": {"description": "intake question"}}),
        json!({"kind": "template", "content": tpl(&aviso, &note_es, &note_pt), "docs": {"description": "hand-off notice"}}),
    ];
    let diff = changes.iter().map(|c| json!({"kind": c["kind"], "id": c["content"]["id"], "version": c["content"]["version"]})).collect();
    let reused: Vec<String> = donor["templates"].as_object().into_iter().flatten().filter_map(|(_, v)| v.as_str().map(str::to_string)).chain(donor["understand"].as_str().map(str::to_string)).collect();
    Ok(Compiled {
        kind: "new_agent".into(),
        target_ref: target.target_ref.clone(),
        agent_id: slug.to_string(),
        changes,
        diff,
        base_digest: format!("donor:{}", catalog.donor),
        cascade: vec![],
        edit_chars: 0,
        edit_budget: 0,
        human_items: vec![
            "admin release settings before prod: interrupt fraude (queue fraude)".into(),
            "admin release settings before prod: injection_ruleset injection-rules@1 and language_detection lang-es-pt@1 (a clone silently loses them)".into(),
            "human approver reads the routing card against the sibling cards (traffic stealing)".into(),
            "separate human promote to prod; first publish lands in staging".into(),
            format!("closure reused unchanged from the donor (the proof copies these live entities into the draft: agent-core needs the full closure of a brand-new agent): {}", reused.join(", ")),
            "eval_suite generated by the engine from the finding (templated es/pt cases plus guards adapted from pulso-min) and proven on the new agent; the human approver reads it. Routing from recepcion to the new agent is NOT proven: the evaluation harness cannot exercise the directory, the agent joins it only with a human promote to prod".into(),
        ],
        expected_effect: effect,
        rationale,
        uncertainty,
    })
}
