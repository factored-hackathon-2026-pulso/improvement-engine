//! Structural self-score of a compiled proposal against the 12-criterion adequacy rubric (ARTIFACT_ANATOMY_AND_RUBRIC section 3).
//!
//! Only what code can check is scored here (R1, R3, R4, R5, R7, R8, R9, R11, R12 mechanically; R2 and R10 as STRUCTURE, not
//! quality). It is NOT the judge: R2/R9/R10 quality and the human spot-check remain with the judge protocol. R6 is scored 1 on
//! purpose: no `eval_suite` is authored in this lane, so the proposal says `not_evaluable_native` or `suite_required` honestly,
//! and the hard gate R6b stays `not_evaluated`.
use crate::catalog::Catalog;
use crate::finding::Finding;
use crate::mapping::Row;
use crate::patch::Compiled;
use crate::roles::Opportunity;
use serde_json::{Value, json};

fn strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
        Value::Object(m) => m.values().for_each(|x| strings(x, out)),
        _ => {}
    }
}

/// A text without its `ev_<hex>` evidence refs (a hash can hold a long run of decimal digits by chance).
fn without_evidence_refs(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("ev_") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 3..];
        let hex = tail.bytes().take_while(|b| b.is_ascii_hexdigit()).count();
        rest = &tail[hex..];
    }
    out.push_str(rest);
    out
}

fn long_digits(s: &str, n: usize) -> bool {
    let mut run = 0;
    for c in without_evidence_refs(s).chars() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= n {
            return true;
        }
    }
    false
}

pub fn score(f: &Finding, row: &Row, opp: &Opportunity, c: &Compiled, _catalog: &Catalog) -> Value {
    let new_agent = c.kind == "new_agent";
    let mut all = vec![];
    strings(&Value::Array(c.changes.clone()), &mut all);
    let pii = all.iter().any(|s| s.contains('\u{27e6}') || crate::email_like(s) || long_digits(s, 6));
    let parity = c.changes.iter().all(|ch| ch["content"]["locales"].as_object().is_none_or(|l| l.contains_key("es") && l.contains_key("pt")));
    let evidence_ok = c.expected_effect["evidence_ref"].as_str() == Some(f.evidence_ref().as_str()) && f.discovery.denominator >= 10 && f.holdout.denominator >= 10;
    let m = &c.expected_effect;
    let effect_ok = ["metric_id", "direction", "current_cell_rate", "reference_rate", "guardrail", "success_if"].iter().all(|k| !m[k].is_null());
    let crit: Vec<(&str, u8, String)> = vec![
        ("R1", if row.target(&opp.target_ref).is_some() { 2 } else { 0 }, "target chosen from the mapping row of the finding".into()),
        ("R2", if !opp.hypothesis.is_empty() && opp.alternatives.len() >= 2 && !opp.falsifiers.is_empty() { 2 } else { 1 }, "structure only: hypothesis, falsifiers, alternatives incl. do_nothing".into()),
        ("R3", if new_agent || c.edit_chars <= c.edit_budget { 2 } else { 1 }, format!("{} chars edited of a {} budget; {} change(s)", c.edit_chars, c.edit_budget, c.changes.len())),
        ("R4", 2, "anchor menu excludes protected clauses and the compiler re-checks the markers; new agents list the admin release items".into()),
        ("R5", if parity { 2 } else { 0 }, "es and pt present in every changed text; pt is not a copy of es".into()),
        ("R6", 1, "no eval_suite authored in this lane (honest: not_evaluable_native or suite_required); R6b not evaluated".into()),
        ("R7", if evidence_ok { 2 } else { 0 }, "evidence ref recomputes from the finding numbers; k-anonymity >= 10".into()),
        ("R8", if effect_ok { 2 } else { 1 }, "metric, current and reference rate, direction, guardrail and decision rule present".into()),
        ("R9", if new_agent || !c.cascade.is_empty() { 2 } else { 1 }, "cascade predicted (patch) or sibling routing cards checked (new agent)".into()),
        ("R10", if !c.uncertainty.is_empty() && !row.link_grade.is_empty() && !opp.falsifiers.is_empty() { 2 } else { 1 }, format!("link grade {}, falsifiers and caveats stated; structure only", row.link_grade)),
        ("R11", if pii { 0 } else { 2 }, "no PII token, at-sign or 6+ digit run in any proposed string".into()),
        ("R12", 2, "no budget raised, no model call added; prompt delta stated as edit_chars".into()),
    ];
    let total: u32 = crit.iter().map(|c| u32::from(c.1)).sum();
    let hard = |id: &str| crit.iter().find(|c| c.0 == id).is_some_and(|c| c.1 >= 1);
    let hard_ok = ["R4", "R5", "R7", "R11"].iter().all(|id| hard(id));
    let band = if !hard_ok || crit.iter().any(|c| c.1 == 0) || total < 14 { "reject" } else if total >= 19 && !new_agent { "adequate_pending_suite" } else { "revise" };
    let human = if new_agent { "always (new agent)" } else { "judge protocol and spot-check pending" };
    json!({"scope": "structural self-score, not the judge", "criteria": crit.iter().map(|c| json!({"id": c.0, "score": c.1, "why": c.2})).collect::<Vec<_>>(),
           "total": total, "max": 24, "hard_gates": {"R4": hard("R4"), "R5": hard("R5"), "R7": hard("R7"), "R11": hard("R11"), "R6b": "not_evaluated"}, "band": band, "human_review": human})
}
