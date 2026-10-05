//! The three roles as `ModelRequest`s plus the strict parsers of their answers.
//!
//! Every payload is a TPS-shaped agent input dict (`goal, inputs, step, tools, observations, output_schema`): opaque ids, enums,
//! small numbers and k-anonymous aggregate rows in `inputs`/`observations`; the only free text is the static, engine-authored
//! anchor menu (base artifact text, no data) carried as tool descriptions. The `registry` of the request lists every token the
//! engine put in `inputs`, so the scanner accepts exactly the closed vocabulary the engine chose.
//!
//! Independence of the Verifier: it gets its own system prompt and payload (the claim as tokens plus the evidence rows; never
//! the Scout's hypothesis text, falsifiers or alternatives) and is called through its own port instance.
use crate::catalog::{Anchor, Artifact, Catalog, anchors};
use crate::clean_text;
use crate::finding::Finding;
use crate::mapping::{MECHANISMS, Row, Target};
use engine::models::{DataClass, ModelRequest, Role};
use serde_json::{Value, json};

pub const SCOUT_SYSTEM: &str = "You are the Scout of an improvement engine for the customer-service agents of a bank. You receive ONE statistical finding as treated aggregates (never raw rows) and a mapping row that lists the real artifacts a change could target. Choose exactly one target_ref from allowed_targets and one mechanism_class from that target's list. State a HYPOTHESIS: what in that artifact could plausibly cause the observed rate (the finding is an association, never a proven cause). Claim the discovery cell rate (window w1) rounded to two decimals. Give one to three falsifiers (observations that would show the hypothesis is wrong) and at least two alternatives, one of them do_nothing, each with why_not. The opportunity id is exactly the literal h_1. Text fields are short plain sentences with no digits, no personal data and no claim about quality or savings. Never invent ids. Answer with ONE JSON object and nothing else (no markdown fence, no commentary), exactly this shape: {\"opportunity\": {\"id\": \"h_1\", \"target_ref\": \"<a target_ref from allowed_targets>\", \"mechanism_class\": \"<a mechanism of that target>\", \"hypothesis\": \"<one sentence>\", \"claimed_rate\": <w1 rate, number between 0 and 1>, \"falsifiers\": [\"<one sentence>\"], \"alternatives\": [{\"kind\": \"do_nothing\", \"why_not\": \"<one sentence>\"}, {\"kind\": \"other_target\", \"why_not\": \"<one sentence>\"}]}}. Closed vocabulary: alternatives kind is do_nothing, other_target or human_owned.";

pub const VERIFIER_SYSTEM: &str = "You are the independent Verifier. You did not write the claim and you do not see how it was reasoned; you see only the claim (target, mechanism, claimed rate) and the treated evidence rows. Run each requested check and answer pass, fail or na: recompute (is claimed_rate equal to the rate of the discovery row w1, two decimals), replication (does the holdout row w2 show the same direction against baseline_rate_holdout), effect_size (is the gap against baseline_rate_discovery material, at least five points and a ratio of 1.25), mechanism_fit (is the target and mechanism plausible for the finding dimensions), dependency (depends_on is none). Then give verdict supported, weakened or refuted and a one-sentence rationale without digits. Answer with ONE JSON object and nothing else (no markdown fence, no commentary), exactly this shape: {\"verdict\": \"supported|weakened|refuted\", \"checks\": [{\"id\": \"recompute\", \"result\": \"pass|fail|na\"}, {\"id\": \"replication\", \"result\": \"...\"}, {\"id\": \"effect_size\", \"result\": \"...\"}, {\"id\": \"mechanism_fit\", \"result\": \"...\"}, {\"id\": \"dependency\", \"result\": \"...\"}], \"rationale\": \"<one sentence, no digits>\"} with each of the five checks exactly once and one value (not the alternatives) in verdict and result.";

pub const BUILDER_SYSTEM: &str = "You are the Builder of an improvement engine for bank customer-service agents. You receive one verified opportunity and, for an existing artifact, a menu of ANCHORS: exact sentences of the real base text (tools whose description lists lines `<anchor id> | <text>`). The ENGINE decides the target, the kind (inputs.kind) and the direction (the metric must go DOWN): you only write the change itself, never a whole text, never the direction, never the target. For kind patch answer patches, each {locale, anchor_id, op, replacement}: op replace swaps the anchor sentence for your replacement, op insert_after keeps the anchor and appends your replacement. Patch EVERY locale in locales (es and pt), at least one and at most max_patches_per_locale per locale, anchor ids only from the menu; the pt replacement is a real Portuguese parallel of the es one, never a copy; keep each replacement short and the total within edit_budget_chars; no digits, no personal data; a placeholder only if it is in placeholders; never touch or restate safety rules (they are not on the menu on purpose). For kind new_agent design a narrow specialist cloned from the donor: agent_id one of slugs; routing summary and examples (two to four) in es and pt that describe ONLY the new topic and cannot steal traffic from the sibling cards in the directory tool; an intake ask and a notice in es and pt (the agent collects the problem, tells the person a human will follow up, and hands off; it has no tools and resolves nothing). If no safe change exists answer kind no_change. Every answer has a rationale (at most 400 characters), at least two alternatives (one is do_nothing) and an uncertainty sentence. Answer with ONE JSON object and nothing else (no markdown fence, no commentary). Shape for patch: {\"proposal\": {\"kind\": \"patch\", \"rationale\": \"<text>\", \"patches\": [{\"locale\": \"es\", \"anchor_id\": \"<id from the es menu>\", \"op\": \"replace\", \"replacement\": \"<es text>\"}, {\"locale\": \"pt\", \"anchor_id\": \"<id from the pt menu>\", \"op\": \"insert_after\", \"replacement\": \"<pt text>\"}], \"alternatives\": [{\"kind\": \"do_nothing\", \"why_not\": \"<text>\"}, {\"kind\": \"other_target\", \"why_not\": \"<text>\"}], \"uncertainty\": \"<text>\"}}. Shape for new_agent: {\"proposal\": {\"kind\": \"new_agent\", \"agent_id\": \"<one of slugs>\", \"rationale\": \"<text>\", \"routing\": {\"summary_es\": \"<text>\", \"summary_pt\": \"<text>\", \"examples_es\": [\"<text>\", \"<text>\"], \"examples_pt\": [\"<text>\", \"<text>\"]}, \"intake\": {\"ask_es\": \"<text>\", \"ask_pt\": \"<text>\", \"notice_es\": \"<text>\", \"notice_pt\": \"<text>\"}, \"alternatives\": [{\"kind\": \"do_nothing\", \"why_not\": \"<text>\"}, {\"kind\": \"other_target\", \"why_not\": \"<text>\"}], \"uncertainty\": \"<text>\"}}. Closed vocabulary: kind is the value of inputs.kind or no_change; op is replace or insert_after; locale is es or pt; alternatives kind is do_nothing, other_target or human_owned. Do not add any other key.";

fn s(x: &Value) -> Vec<String> {
    let mut out = vec![];
    fn walk(x: &Value, out: &mut Vec<String>) {
        match x {
            Value::String(t) => out.push(t.clone()),
            Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            Value::Object(m) => m.values().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    walk(x, &mut out);
    out
}

fn request(role: Role, system: &str, goal: &str, inputs: Value, tools: Value, observations: Value, schema: Value, dc: DataClass) -> ModelRequest {
    let mut registry = s(&inputs);
    registry.sort();
    registry.dedup();
    ModelRequest {
        role,
        system: system.into(),
        payload: json!({"goal": goal, "inputs": inputs, "step": 0, "tools": tools, "observations": observations, "output_schema": schema}),
        registry,
        data_class: dc,
    }
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "required": required, "additionalProperties": false, "properties": props})
}

fn text() -> Value {
    json!({"type": "string"})
}

fn alternatives_schema() -> Value {
    json!({"type": "array", "items": obj(json!({"kind": {"type": "string", "enum": ["do_nothing", "other_target", "human_owned"]}, "why_not": text()}), &["kind", "why_not"])})
}

fn targets_input(row: &Row) -> Value {
    Value::Array(row.targets.iter().map(|t| json!({"target_ref": t.target_ref, "kind": t.kind, "agent": t.agent, "mechanisms": t.mechanisms, "rank": t.rank})).collect())
}

pub fn scout_request(f: &Finding, row: &Row, dc: DataClass) -> ModelRequest {
    let mut inputs = f.inputs();
    inputs["mapping_row"] = json!(row.id);
    inputs["allowed_targets"] = targets_input(row);
    inputs["link_grade"] = json!(row.link_grade);
    inputs["caveats"] = json!(row.caveats);
    inputs["topic"] = json!(row.topic);
    inputs["mapping_claim"] = json!("hypothesis_of_where_to_intervene_not_a_cause");
    inputs["claim_window"] = json!("w1");
    inputs["opportunity_id"] = json!("h_1");
    let schema = obj(
        json!({"opportunity": obj(
            json!({"id": text(), "target_ref": text(), "mechanism_class": {"type": "string", "enum": MECHANISMS}, "hypothesis": text(), "claimed_rate": {"type": "number"},
                   "falsifiers": {"type": "array", "items": text()}, "alternatives": alternatives_schema()}),
            &["id", "target_ref", "mechanism_class", "hypothesis", "claimed_rate", "falsifiers", "alternatives"])}),
        &["opportunity"],
    );
    request(Role::Scout, SCOUT_SYSTEM, "state one opportunity for the finding", inputs, json!([]), f.observations(), schema, dc)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alt {
    pub kind: String,
    pub why_not: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Opportunity {
    pub id: String,
    pub target_ref: String,
    pub mechanism_class: String,
    pub hypothesis: String,
    pub claimed_rate: f64,
    pub falsifiers: Vec<String>,
    pub alternatives: Vec<Alt>,
}

impl Opportunity {
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "target_ref": self.target_ref, "mechanism_class": self.mechanism_class, "hypothesis": self.hypothesis, "claimed_rate": self.claimed_rate,
               "falsifiers": self.falsifiers, "alternatives": self.alternatives.iter().map(|a| json!({"kind": a.kind, "why_not": a.why_not})).collect::<Vec<_>>()})
    }
}

fn exact_keys(v: &Value, allowed: &[&str], what: &str) -> Result<(), String> {
    let o = v.as_object().ok_or_else(|| format!("{what} is not an object"))?;
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("{what} has an unexpected key {k:?}")),
        None => Ok(()),
    }
}

fn alts(v: &Value) -> Result<Vec<Alt>, String> {
    let arr = v.as_array().ok_or("alternatives is not a list")?;
    let mut out = vec![];
    for a in arr {
        exact_keys(a, &["kind", "why_not"], "alternative")?;
        let kind = a["kind"].as_str().filter(|k| ["do_nothing", "other_target", "human_owned"].contains(k)).ok_or("alternative kind is not in the closed set")?;
        let why = a["why_not"].as_str().ok_or("alternative without why_not")?;
        clean_text(why, 240).map_err(|e| format!("alternative why_not: {e}"))?;
        out.push(Alt { kind: kind.into(), why_not: why.into() });
    }
    if out.len() < 2 || !out.iter().any(|a| a.kind == "do_nothing") {
        return Err("at least two alternatives, one of them do_nothing".into());
    }
    Ok(out)
}

pub fn parse_scout(row: &Row, answer: &Value) -> Result<Opportunity, String> {
    exact_keys(answer, &["opportunity"], "scout answer")?;
    let o = &answer["opportunity"];
    exact_keys(o, &["id", "target_ref", "mechanism_class", "hypothesis", "claimed_rate", "falsifiers", "alternatives"], "opportunity")?;
    let id = o["id"].as_str().filter(|i| i.strip_prefix("h_").is_some_and(|d| (1..=4).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()))).ok_or("opportunity id is not h_<n>")?;
    let target_ref = o["target_ref"].as_str().ok_or("opportunity without target_ref")?;
    let target: &Target = row.target(target_ref).ok_or_else(|| format!("target_ref {target_ref:?} is not in the mapping row"))?;
    let mech = o["mechanism_class"].as_str().ok_or("opportunity without mechanism_class")?;
    if !target.mechanisms.contains(&mech) {
        return Err(format!("mechanism_class {mech:?} is not allowed for {target_ref}"));
    }
    let hypothesis = o["hypothesis"].as_str().ok_or("opportunity without hypothesis")?;
    clean_text(hypothesis, 500).map_err(|e| format!("hypothesis: {e}"))?;
    let claimed_rate = o["claimed_rate"].as_f64().filter(|r| (0.0..=1.0).contains(r)).ok_or("claimed_rate is not a number in 0..1")?;
    let falsifiers: Vec<String> = o["falsifiers"].as_array().ok_or("falsifiers is not a list")?.iter().map(|x| x.as_str().map(str::to_string).ok_or("falsifier is not text")).collect::<Result<_, _>>()?;
    if falsifiers.is_empty() || falsifiers.len() > 3 {
        return Err("one to three falsifiers".into());
    }
    for fl in &falsifiers {
        clean_text(fl, 240).map_err(|e| format!("falsifier: {e}"))?;
    }
    Ok(Opportunity { id: id.into(), target_ref: target_ref.into(), mechanism_class: mech.into(), hypothesis: hypothesis.into(), claimed_rate, falsifiers, alternatives: alts(&o["alternatives"])? })
}

pub const CHECK_IDS: [&str; 5] = ["recompute", "replication", "effect_size", "mechanism_fit", "dependency"];

pub fn verifier_request(f: &Finding, opp: &Opportunity, dc: DataClass) -> ModelRequest {
    let mut inputs = f.inputs();
    inputs["hypothesis_id"] = json!(opp.id);
    inputs["claim"] = json!({"target_ref": opp.target_ref, "mechanism_class": opp.mechanism_class, "claimed_rate": crate::finding::round2(opp.claimed_rate), "claim_window": "w1"});
    inputs["checks_requested"] = json!(CHECK_IDS);
    let schema = obj(
        json!({"verdict": {"type": "string", "enum": ["supported", "weakened", "refuted"]},
               "checks": {"type": "array", "items": obj(json!({"id": {"type": "string", "enum": CHECK_IDS}, "result": {"type": "string", "enum": ["pass", "fail", "na"]}}), &["id", "result"])},
               "rationale": text()}),
        &["verdict", "checks", "rationale"],
    );
    request(Role::Verifier, VERIFIER_SYSTEM, "verify the claim against the evidence", inputs, json!([]), f.observations(), schema, dc)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelVerdict {
    pub verdict: String,
    pub checks: Vec<(String, String)>,
    pub rationale: String,
}

pub fn parse_verifier(answer: &Value) -> Result<ModelVerdict, String> {
    exact_keys(answer, &["verdict", "checks", "rationale"], "verifier answer")?;
    let verdict = answer["verdict"].as_str().filter(|v| ["supported", "weakened", "refuted"].contains(v)).ok_or("verdict is not supported|weakened|refuted")?;
    let mut checks = vec![];
    for c in answer["checks"].as_array().ok_or("checks is not a list")? {
        exact_keys(c, &["id", "result"], "check")?;
        let id = c["id"].as_str().filter(|i| CHECK_IDS.contains(i)).ok_or("unknown check id")?;
        let r = c["result"].as_str().filter(|r| ["pass", "fail", "na"].contains(r)).ok_or("check result is not pass|fail|na")?;
        checks.push((id.to_string(), r.to_string()));
    }
    for id in CHECK_IDS {
        if checks.iter().filter(|c| c.0 == id).count() != 1 {
            return Err(format!("check {id} must appear exactly once"));
        }
    }
    let rationale = answer["rationale"].as_str().ok_or("verifier without rationale")?;
    clean_text(rationale, 400).map_err(|e| format!("rationale: {e}"))?;
    Ok(ModelVerdict { verdict: verdict.into(), checks, rationale: rationale.into() })
}

/// Edit budget in characters for a patch of `art` (rubric R3): the larger of 240 and a quarter of the longest locale text.
pub fn edit_budget(art: &Artifact) -> usize {
    let longest = art.locales.values().map(|t| t.chars().count()).max().unwrap_or(0);
    (longest / 4).clamp(240, 1000)
}

fn anchor_tools(art: &Artifact, locales: &[&str]) -> Vec<Value> {
    let mut tools = vec![];
    for l in locales {
        let menu: Vec<Anchor> = anchors(art, l);
        let (mut chunk, mut n) = (String::new(), 1);
        let flush = |chunk: &mut String, n: &mut usize, tools: &mut Vec<Value>| {
            if !chunk.is_empty() {
                tools.push(json!({"tool": format!("pulso/anchors_{l}_{n}@1.0.0"), "description": chunk.trim_end(), "args_schema": {"type": "object"}}));
                chunk.clear();
                *n += 1;
            }
        };
        for a in &menu {
            let line = format!("{} | {}\n", a.id, a.text);
            if chunk.len() + line.len() > 1800 {
                flush(&mut chunk, &mut n, &mut tools);
            }
            chunk.push_str(&line);
        }
        flush(&mut chunk, &mut n, &mut tools);
    }
    tools
}

fn directory_tool(catalog: &Catalog) -> Value {
    let mut lines = String::new();
    for id in catalog.agent_ids() {
        let a = catalog.agent(&id).cloned().unwrap_or(Value::Null);
        if let Some(r) = a.get("routing").filter(|r| r.is_object()) {
            let ex: Vec<String> = r["examples"].as_array().into_iter().flatten().filter_map(|e| e.as_str().map(str::to_string)).collect();
            lines.push_str(&format!("{id} | {} | examples: {}\n", r["summary"].as_str().unwrap_or(""), ex.join(" ; ")));
        }
    }
    json!({"tool": "pulso/directory_cards@1.0.0", "description": lines.trim_end(), "args_schema": {"type": "object"}})
}

pub const LOCALES: [&str; 2] = ["es", "pt"];

pub fn builder_request(f: &Finding, opp: &Opportunity, row: &Row, catalog: &Catalog, dc: DataClass) -> Result<ModelRequest, String> {
    let target = row.target(&opp.target_ref).ok_or("opportunity target is not in the mapping row")?;
    let mut inputs = f.inputs();
    inputs["hypothesis_id"] = json!(opp.id);
    inputs["target_ref"] = json!(opp.target_ref);
    inputs["mechanism_class"] = json!(opp.mechanism_class);
    inputs["kind"] = json!(target.kind);
    inputs["agent"] = json!(target.agent);
    inputs["link_grade"] = json!(row.link_grade);
    inputs["locales"] = json!(LOCALES);
    inputs["ops"] = json!(["replace", "insert_after"]);
    inputs["max_patches_per_locale"] = json!(2);
    inputs["placeholders"] = json!(target.placeholders);
    inputs["slugs"] = json!(target.slugs);
    inputs["donor"] = json!(catalog.donor);
    let mut tools = vec![];
    if target.kind == "patch" {
        let art = catalog.get(&opp.target_ref).ok_or_else(|| format!("{} is not in the baseline catalogue", opp.target_ref))?;
        inputs["edit_budget_chars"] = json!(edit_budget(art));
        tools = anchor_tools(art, &LOCALES);
    } else {
        inputs["limits"] = json!({"summary_max_chars": 240, "example_max_chars": 100, "example_min": 2, "example_max": 4, "text_max_chars": 240});
        tools.push(directory_tool(catalog));
    }
    let patch = obj(json!({"locale": {"type": "string", "enum": LOCALES}, "anchor_id": text(), "op": {"type": "string", "enum": ["replace", "insert_after"]}, "replacement": text()}), &["locale", "anchor_id", "op", "replacement"]);
    let schema = obj(
        json!({"proposal": obj(
            json!({"kind": {"type": "string", "enum": [target.kind, "no_change"]}, "rationale": text(),
                   "patches": {"type": "array", "items": patch}, "agent_id": text(),
                   "routing": obj(json!({"summary_es": text(), "summary_pt": text(), "examples_es": {"type": "array", "items": text()}, "examples_pt": {"type": "array", "items": text()}}), &[]),
                   "intake": obj(json!({"ask_es": text(), "ask_pt": text(), "notice_es": text(), "notice_pt": text()}), &[]),
                   "alternatives": alternatives_schema(), "uncertainty": text()}),
            &["kind", "rationale", "alternatives", "uncertainty"])}),
        &["proposal"],
    );
    Ok(request(Role::Builder, BUILDER_SYSTEM, "propose one change as anchored patches or one new agent", inputs, Value::Array(tools), json!([]), schema, dc))
}

/// Schema-level parse only; the semantic checks (anchors, locales, protected clauses, PII) are `patch::compile`.
pub fn parse_builder(answer: &Value) -> Result<Value, String> {
    exact_keys(answer, &["proposal"], "builder answer")?;
    let p = &answer["proposal"];
    exact_keys(p, &["kind", "target_ref", "rationale", "patches", "agent_id", "routing", "intake", "expected_direction", "alternatives", "uncertainty"], "proposal")?;
    p["kind"].as_str().filter(|k| ["patch", "new_agent", "no_change"].contains(k)).ok_or("proposal kind is not patch|new_agent|no_change")?;
    clean_text(p["rationale"].as_str().ok_or("proposal without rationale")?, 600).map_err(|e| format!("rationale: {e}"))?;
    clean_text(p["uncertainty"].as_str().ok_or("proposal without uncertainty")?, 400).map_err(|e| format!("uncertainty: {e}"))?;
    // `target_ref` and `expected_direction` are tolerated and IGNORED: the engine derives both (BLD1), the model cannot fail a proposal over them.
    alts(&p["alternatives"])?;
    Ok(p.clone())
}
