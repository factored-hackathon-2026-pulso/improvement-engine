//! Scout -> Verifier -> Builder over one finding, on three `ModelPort`s.
//!
//! Stops are typed and final for the finding: `unlinked` (no mapping row), `blocked(<reason>)` (a port refused or failed, an answer
//! was invalid, the deterministic recompute refuted the claim, the independent verifier refuted it, the compiler denied the
//! proposal). There is no fallback to another port and no retry with a different prompt. Every model call is recorded under the
//! label and model id of the port that handled it; `doubles[]` lists every call that was not an answered gateway call.
use crate::catalog::Catalog;
use crate::finding::{Finding, Source, deterministic_checks};
use crate::mapping::{Row, map_finding};
use crate::patch::compile;
use crate::roles::{self, ModelVerdict, Opportunity};
use crate::rubric;
use engine::models::{DataClass, Label, ModelError, ModelPort, ModelRequest, Recording, Role};
use serde_json::{Value, json};
use std::rc::Rc;

pub struct Ports {
    pub scout: Rc<dyn ModelPort>,
    pub verifier: Rc<dyn ModelPort>,
    pub builder: Rc<dyn ModelPort>,
    /// Stronger Builder tier (user policy: pro for tasks that need more reasoning). Used ONLY when the primary Builder could not
    /// produce a compiled proposal after its bounded retries; the tier that answered is named in the outcome.
    pub builder_escalation: Option<Rc<dyn ModelPort>>,
}

impl Ports {
    pub fn new(scout: Rc<dyn ModelPort>, verifier: Rc<dyn ModelPort>, builder: Rc<dyn ModelPort>) -> Ports {
        Ports { scout, verifier, builder, builder_escalation: None }
    }
}

/// Retries after a first answer that does not parse or compile, each with the problem fed back (bounded: never more than 2).
pub const MAX_RETRIES: usize = 2;

/// `flash | pro | other` from the model id (the tier label that travels in every outcome).
pub fn tier_of(model_id: &str) -> &'static str {
    let m = model_id.rsplit('/').next().unwrap_or(model_id);
    if m.ends_with("-flash") || m.contains("flash") {
        "flash"
    } else if m.ends_with("-pro") || m.contains("-pro") {
        "pro"
    } else {
        "other"
    }
}

/// A rejected answer: (stage, closed reason code, why).
type Reject = (&'static str, String, String);

/// ASCII-only, short, no model free text beyond ids: what is fed back must itself pass the treated-payload scan.
fn feedback_text(problem: &str) -> String {
    let clean: String = problem.chars().map(|c| if c.is_ascii_alphanumeric() || " _.,:;/'()-".contains(c) { c } else { '?' }).take(380).collect();
    clean.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `answer_not_json:<code>` (the extractor's typed failure) as a sentence the model can act on.
fn packaging_hint(w: &str) -> String {
    match w.strip_prefix("answer_not_json:") {
        Some("empty") => "the answer was empty".into(),
        Some("no_json") => "the answer had no JSON object".into(),
        Some("truncated") => "the answer was cut off before the JSON object closed: write shorter texts".into(),
        Some("unparseable") => "the JSON is malformed: every key and every string needs double quotes and items need commas".into(),
        Some("not_object") => "the answer is JSON but not an object".into(),
        _ => w.to_string(),
    }
}

enum Asked<T> {
    Done(T),
    /// The port refused or was unavailable: final, typed, no retry.
    Stopped(ModelError),
    /// Every attempt produced an answer that did not survive `check`: the last rejection.
    Rejected(Reject),
}

/// One role call with up to `MAX_RETRIES` retries. A retry is the SAME request plus the previous problem as `feedback`. Returns how
/// many calls were made.
fn ask<T>(port: &Recording, req: &ModelRequest, stage: &'static str, check: &dyn Fn(&Value) -> Result<T, Reject>) -> (Asked<T>, usize) {
    let mut last: Option<Reject> = None;
    for attempt in 0..=MAX_RETRIES {
        let mut r = req.clone();
        if let Some((_, _, why)) = &last {
            r.payload["feedback"] = json!(format!("Attempt {attempt} was rejected: {}. Answer again with ONE JSON object that follows output_schema exactly.", feedback_text(why)));
        }
        match port.call(&r) {
            Err(ModelError::Invalid(w)) => last = Some((stage, "model_invalid".into(), packaging_hint(&w))),
            Err(e) => return (Asked::Stopped(e), attempt + 1),
            Ok(a) => match check(&a.content) {
                Ok(t) => return (Asked::Done(t), attempt + 1),
                Err(rej) => last = Some(rej),
            },
        }
    }
    (Asked::Rejected(last.expect("at least one attempt")), MAX_RETRIES + 1)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Opts {
    /// Explicit opt-in: treated aggregates DERIVED from real data (bank cells, E0) may reach a real hosted model. Without it
    /// the run stops before any call. Synthetic data never needs it.
    pub allow_derived_aggregates: bool,
}

#[derive(Debug, Clone)]
pub struct Reasoned {
    pub finding_id: String,
    /// `proposed | no_change | unlinked | blocked`
    pub status: String,
    pub reason: String,
    pub stage: String,
    pub detail: String,
    pub mapping_row: Option<String>,
    pub opportunity: Option<Value>,
    pub verification: Option<Value>,
    pub compiled: Option<Value>,
    /// The typed compile result (what a writer delivers); `compiled` is its JSON.
    pub compiled_raw: Option<crate::patch::Compiled>,
    pub rubric: Option<Value>,
    pub calls: Vec<Value>,
    pub doubles: Vec<Value>,
    pub independence: Value,
    pub source: &'static str,
    /// Cost, tokens, latency and attempts of every model call of this finding, and the Builder tier that answered.
    pub metering: Value,
}

impl Reasoned {
    pub fn to_json(&self) -> Value {
        json!({"finding_id": self.finding_id, "status": self.status, "reason": self.reason, "stage": self.stage, "detail": self.detail, "mapping_row": self.mapping_row,
               "source": self.source, "opportunity": self.opportunity, "verification": self.verification, "proposal": self.compiled, "rubric": self.rubric,
               "model_calls": self.calls, "doubles": self.doubles, "independence": self.independence, "metering": self.metering})
    }
}

fn blocked(mut r: Reasoned, reason: &str, stage: &str, detail: impl Into<String>) -> Reasoned {
    r.status = "blocked".into();
    r.reason = reason.into();
    r.stage = stage.into();
    r.detail = detail.into().chars().take(300).collect();
    r
}

fn model_stop(r: Reasoned, stage: &str, e: ModelError) -> Reasoned {
    match e {
        ModelError::Refused(w) => blocked(r, "model_refused", stage, w),
        ModelError::Unavailable(w) => blocked(r, "model_unavailable", stage, w),
        ModelError::Invalid(w) => blocked(r, "model_invalid", stage, w),
    }
}

/// Final verification status: the model can weaken or refute, the code can only hold the line (a deterministic hard failure
/// refutes whatever the model said; a soft failure or a model-side `fail` weakens).
pub fn combine(det: &[crate::finding::Check], m: &ModelVerdict) -> (&'static str, Vec<String>) {
    let mut notes = vec![];
    let hard_fail = det.iter().any(|c| ["recompute", "k_anonymity", "replication"].contains(&c.id) && c.result == "fail");
    let soft_fail = det.iter().any(|c| c.result == "fail" && !["recompute", "k_anonymity", "replication"].contains(&c.id));
    for c in det {
        if let Some((_, mr)) = m.checks.iter().find(|x| x.0 == c.id)
            && mr != c.result
            && c.result != "na"
        {
            notes.push(format!("check {} differs: code {}, model {mr}", c.id, c.result));
        }
    }
    let status = if hard_fail || m.verdict == "refuted" {
        "refuted"
    } else if soft_fail || m.verdict == "weakened" || m.checks.iter().any(|c| c.1 == "fail") {
        "weakened"
    } else {
        "supported"
    };
    (status, notes)
}

/// `other_family` (different vendor), `same_family_other_tier` (same vendor prefix, other model), else prompt and context only.
pub fn independence_level(scout: &str, verifier: &str) -> &'static str {
    if scout == verifier {
        "separate_prompt_and_context_only"
    } else if scout.split('/').next() == verifier.split('/').next() {
        "same_family_other_tier"
    } else {
        "other_family"
    }
}

pub fn reason(catalog: &Catalog, f: &Finding, ports: &Ports, opts: &Opts) -> Reasoned {
    let (rs, rv, rb) = (Rc::new(Recording::new(ports.scout.clone())), Rc::new(Recording::new(ports.verifier.clone())), Rc::new(Recording::new(ports.builder.clone())));
    let rbe = ports.builder_escalation.as_ref().map(|p| Rc::new(Recording::new(p.clone())));
    let mut r = Reasoned {
        finding_id: f.id.clone(), status: "blocked".into(), reason: String::new(), stage: String::new(), detail: String::new(), mapping_row: None, opportunity: None, verification: None,
        compiled: None, compiled_raw: None, rubric: None, calls: vec![], doubles: vec![], source: f.source.as_str(), metering: Value::Null,
        independence: json!({"scout_model": ports.scout.model_id(), "verifier_model": ports.verifier.model_id(), "builder_model": ports.builder.model_id(),
                             "builder_escalation_model": ports.builder_escalation.as_ref().map(|p| p.model_id()),
                             "verifier_separate_port": !Rc::ptr_eq(&ports.scout, &ports.verifier),
                             "level": independence_level(&ports.scout.model_id(), &ports.verifier.model_id())}),
    };
    let attempts = std::cell::RefCell::new(json!({"scout": 0, "verifier": 0, "builder": 0, "builder_escalation": 0}));
    let tier_used: std::cell::RefCell<Option<(String, String, bool)>> = std::cell::RefCell::new(None);
    let finish = |mut r: Reasoned| {
        let all: Vec<&Rc<Recording>> = [Some(&rs), Some(&rv), Some(&rb), rbe.as_ref()].into_iter().flatten().collect();
        r.calls = all.iter().flat_map(|x| x.calls().iter().map(|c| c.to_json()).collect::<Vec<_>>()).collect();
        r.doubles = all.iter().flat_map(|x| x.doubles()).collect();
        let recs: Vec<engine::models::CallRecord> = all.iter().flat_map(|x| x.calls()).collect();
        let usage = |f: &dyn Fn(&engine::models::Usage) -> f64| recs.iter().filter_map(|c| c.usage.as_ref()).map(f).sum::<f64>();
        let tu = tier_used.borrow().clone();
        r.metering = json!({
            "calls": recs.len(), "attempts": attempts.borrow().clone(),
            "tokens_in": usage(&|u| u.tokens_in as f64) as u64, "tokens_out": usage(&|u| u.tokens_out as f64) as u64,
            "cost_usd": (usage(&|u| u.cost_f64()) * 1e6).round() / 1e6,
            "latency_ms": recs.iter().map(|c| c.wall_ms).sum::<u64>(),
            "builder": tu.map(|(tier, model, escalated)| json!({"tier": tier, "model": model, "escalated": escalated})),
        });
        r
    };
    if f.direction != "up" {
        // Every cells metric is higher-is-worse: a cell BELOW its reference is a good result, not an opportunity. No model is called.
        r.status = "no_change".into();
        r.reason = "better_than_reference".into();
        r.stage = "direction".into();
        r.detail = format!("metric {} is {} against the reference in this cell: nothing to improve", f.metric, f.direction);
        return finish(r);
    }
    let Some(row) = map_finding(f) else {
        let (code, why) = crate::mapping::unlinked_reason(f);
        r.status = "unlinked".into();
        r.reason = code.into();
        r.stage = "mapping".into();
        r.detail = why;
        return finish(r);
    };
    r.mapping_row = Some(row.id.into());
    let hosted = [&ports.scout, &ports.verifier, &ports.builder].iter().any(|p| p.label() == Label::Gateway);
    if f.source.derived() && hosted && !opts.allow_derived_aggregates {
        return finish(blocked(r, "derived_data_opt_in_required", "policy", format!("{} aggregates reach a real hosted model only with the explicit opt-in flag", f.source.as_str())));
    }
    // After the opt-in (or for synthetic data) the payload is a treated aggregate: the scanner of each port is the gate.
    let dc = if f.source == Source::Synthetic { DataClass::Synthetic } else { DataClass::Treated };

    // 1. Scout (bounded retries with the parse problem fed back)
    let (got, n) = ask(&rs, &roles::scout_request(f, &row, dc), "scout", &|a| roles::parse_scout(&row, a).map_err(|e| ("scout", "model_invalid".to_string(), e)));
    attempts.borrow_mut()["scout"] = json!(n);
    let opp: Opportunity = match got {
        Asked::Done(o) => o,
        Asked::Stopped(e) => return finish(model_stop(r, "scout", e)),
        Asked::Rejected((st, code, why)) => return finish(blocked(r, &code, st, why)),
    };
    r.opportunity = Some(opp.to_json());

    // 2. deterministic recompute and structural checks (the model cannot overrule them)
    let det = deterministic_checks(f, opp.claimed_rate);
    let det_json: Vec<Value> = det.iter().map(|c| json!({"id": c.id, "result": c.result})).collect();
    if det.iter().any(|c| c.id == "recompute" && c.result == "fail") {
        r.verification = Some(json!({"status": "refuted", "deterministic": det_json, "model": null}));
        return finish(blocked(r, "claim_not_corroborated", "recompute", "the claimed rate does not recompute from the evidence numbers (two decimals)"));
    }

    // 3. independent Verifier
    let (got, n) = ask(&rv, &roles::verifier_request(f, &opp, dc), "verifier", &|a| roles::parse_verifier(a).map_err(|e| ("verifier", "model_invalid".to_string(), e)));
    attempts.borrow_mut()["verifier"] = json!(n);
    let mv = match got {
        Asked::Done(v) => v,
        Asked::Stopped(e) => return finish(model_stop(r, "verifier", e)),
        Asked::Rejected((st, code, why)) => return finish(blocked(r, &code, st, why)),
    };
    let (status, notes) = combine(&det, &mv);
    r.verification = Some(json!({"status": status, "deterministic": det_json, "model": {"verdict": mv.verdict, "checks": mv.checks.iter().map(|c| json!({"id": c.0, "result": c.1})).collect::<Vec<_>>(), "rationale": mv.rationale},
                                 "disagreements": notes, "human_review_required": status == "weakened"}));
    if status == "refuted" {
        return finish(blocked(r, "verifier_refuted", "verifier", mv.rationale));
    }

    // 4. Builder + byte-exact compile. The parse AND the compile are the check: a denial is fed back like a parse problem. When the
    // primary tier cannot produce a compiled proposal after its retries and an escalation tier is configured, that tier tries once
    // more (same bounded retries) and the outcome names the tier that answered.
    let req = match roles::builder_request(f, &opp, &row, catalog, dc) {
        Ok(q) => q,
        Err(e) => return finish(blocked(r, "precondition_missing", "builder", e)),
    };
    let check = |a: &Value| -> Result<crate::patch::Compiled, Reject> {
        let p = roles::parse_builder(a).map_err(|e| ("builder", "model_invalid".to_string(), e))?;
        compile(catalog, f, &row, &opp, &p).map_err(|d| ("compile", format!("compile_denied:{}", d.code), d.why))
    };
    let (got, n) = ask(&rb, &req, "builder", &check);
    attempts.borrow_mut()["builder"] = json!(n);
    *tier_used.borrow_mut() = Some((tier_of(&ports.builder.model_id()).into(), ports.builder.model_id(), false));
    let got = match (got, &rbe) {
        (Asked::Rejected(_), Some(esc)) => {
            let (g2, n2) = ask(esc, &req, "builder", &check);
            attempts.borrow_mut()["builder_escalation"] = json!(n2);
            let em = ports.builder_escalation.as_ref().map(|p| p.model_id()).unwrap_or_default();
            *tier_used.borrow_mut() = Some((tier_of(&em).into(), em, true));
            g2
        }
        (g, _) => g,
    };
    match got {
        Asked::Stopped(e) => finish(model_stop(r, "builder", e)),
        Asked::Rejected((st, code, why)) => finish(blocked(r, &code, st, why)),
        Asked::Done(c) => {
            r.rubric = (c.kind != "no_change").then(|| rubric::score(f, &row, &opp, &c, catalog));
            r.status = if c.kind == "no_change" { "no_change" } else { "proposed" }.into();
            r.reason = if c.kind == "no_change" { "builder_no_safe_change" } else { "compiled" }.into();
            r.stage = "compile".into();
            r.detail = c.rationale.chars().take(300).collect();
            r.compiled = Some(c.to_json());
            r.compiled_raw = Some(c);
            finish(r)
        }
    }
}

/// Convenience for callers that hold the sensor report: every corroborated signal is reasoned over, the others are listed.
pub fn reason_report(catalog: &Catalog, report: &Value, source: Source, ports: &Ports, opts: &Opts) -> Result<Value, String> {
    let (findings, skipped) = Finding::from_report(report, source)?;
    let results: Vec<Reasoned> = findings.iter().map(|f| reason(catalog, f, ports, opts)).collect();
    let mut doubles: Vec<Value> = vec![];
    for r in &results {
        doubles.extend(r.doubles.clone());
    }
    let n = |s: &str| results.iter().filter(|r| r.status == s).count();
    let mut by_reason: serde_json::Map<String, Value> = serde_json::Map::new();
    for r in results.iter().filter(|r| r.status == "unlinked") {
        let c = by_reason.get(&r.reason).and_then(Value::as_u64).unwrap_or(0) + 1;
        by_reason.insert(r.reason.clone(), json!(c));
    }
    let _ = Role::Scout;
    Ok(json!({
        "contract": "reasoning-run/l2-0", "baseline": {"label": catalog.label, "source": catalog.source}, "sensor_semantics": report["semantics"], "data_source": source.as_str(),
        "quality_claims": "forbidden", "opt_in_derived_aggregates": opts.allow_derived_aggregates,
        "summary": {"corroborated": findings.len(), "proposed": n("proposed"), "no_change": n("no_change"), "unlinked": n("unlinked"), "blocked": n("blocked"), "skipped_not_corroborated": skipped.len(),
                    // an unlinked finding is descriptive (explicit reason), never a failure: only `blocked` counts against the roles
                    "unlinked_by_reason": by_reason, "failed": n("blocked"), "linked": findings.len() - n("unlinked") - n("no_change") + 0,
                    "cost_usd": (results.iter().map(|r| r.metering["cost_usd"].as_f64().unwrap_or(0.0)).sum::<f64>() * 1e6).round() / 1e6},
        "skipped": skipped.iter().map(|s| json!({"index": s.index, "metric": s.metric, "status": s.status, "reason": s.reason})).collect::<Vec<_>>(),
        "findings": results.iter().map(Reasoned::to_json).collect::<Vec<_>>(),
        "doubles": doubles,
    }))
}

pub fn row_of(f: &Finding) -> Option<Row> {
    map_finding(f)
}
