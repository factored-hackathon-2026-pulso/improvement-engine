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
use engine::models::{DataClass, Label, ModelError, ModelPort, Recording, Role};
use serde_json::{Value, json};
use std::rc::Rc;

pub struct Ports {
    pub scout: Rc<dyn ModelPort>,
    pub verifier: Rc<dyn ModelPort>,
    pub builder: Rc<dyn ModelPort>,
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
    pub rubric: Option<Value>,
    pub calls: Vec<Value>,
    pub doubles: Vec<Value>,
    pub independence: Value,
    pub source: &'static str,
    /// Decision dossier (W1-2) of a proposed finding, built without a regression verdict: `announce` stays false until the REG1
    /// verdict story is supplied (`reason_cli dossier --verdict`), the dossier says so. `{"error": ..}` if its PII guard refused.
    pub dossier: Option<Value>,
}

impl Reasoned {
    pub fn to_json(&self) -> Value {
        json!({"finding_id": self.finding_id, "status": self.status, "reason": self.reason, "stage": self.stage, "detail": self.detail, "mapping_row": self.mapping_row,
               "source": self.source, "opportunity": self.opportunity, "verification": self.verification, "proposal": self.compiled, "rubric": self.rubric,
               "model_calls": self.calls, "doubles": self.doubles, "independence": self.independence, "dossier": self.dossier})
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

pub fn reason(catalog: &Catalog, f: &Finding, ports: &Ports, opts: &Opts) -> Reasoned {
    let (rs, rv, rb) = (Rc::new(Recording::new(ports.scout.clone())), Rc::new(Recording::new(ports.verifier.clone())), Rc::new(Recording::new(ports.builder.clone())));
    let mut r = Reasoned {
        finding_id: f.id.clone(), status: "blocked".into(), reason: String::new(), stage: String::new(), detail: String::new(), mapping_row: None, opportunity: None, verification: None,
        compiled: None, rubric: None, calls: vec![], doubles: vec![], source: f.source.as_str(), dossier: None,
        independence: json!({"scout_model": ports.scout.model_id(), "verifier_model": ports.verifier.model_id(), "builder_model": ports.builder.model_id(),
                             "verifier_separate_port": !Rc::ptr_eq(&ports.scout, &ports.verifier),
                             "level": if ports.scout.model_id() != ports.verifier.model_id() { "other_model" } else { "separate_prompt_and_context_only" }}),
    };
    let finish = |mut r: Reasoned| {
        r.calls = [&rs, &rv, &rb].iter().flat_map(|x| x.calls().iter().map(|c| c.to_json()).collect::<Vec<_>>()).collect();
        r.doubles = [&rs, &rv, &rb].iter().flat_map(|x| x.doubles()).collect();
        if r.status == "proposed" {
            let real = [&ports.scout, &ports.verifier, &ports.builder].iter().all(|p| p.label() == Label::Gateway);
            let labels = crate::dossier::Labels { runtime: if real { crate::dossier::Runtime::Real } else { crate::dossier::Runtime::Doubles }, ..Default::default() };
            let record = json!({"proposal": r.compiled, "doubles": r.doubles, "rubric": r.rubric});
            r.dossier = Some(crate::dossier::build(&f.to_signal_json(), &record, None, &labels).unwrap_or_else(|e| json!({"error": e})));
        }
        r
    };
    let Some(row) = map_finding(f) else {
        r.status = "unlinked".into();
        r.reason = "no_mapping".into();
        r.stage = "mapping".into();
        r.detail = format!("metric {} with these dimensions maps to no agent-core artifact: descriptive finding, no proposal", f.metric);
        return finish(r);
    };
    r.mapping_row = Some(row.id.into());
    let hosted = [&ports.scout, &ports.verifier, &ports.builder].iter().any(|p| p.label() == Label::Gateway);
    if f.source.derived() && hosted && !opts.allow_derived_aggregates {
        return finish(blocked(r, "derived_data_opt_in_required", "policy", format!("{} aggregates reach a real hosted model only with the explicit opt-in flag", f.source.as_str())));
    }
    // After the opt-in (or for synthetic data) the payload is a treated aggregate: the scanner of each port is the gate.
    let dc = if f.source == Source::Synthetic { DataClass::Synthetic } else { DataClass::Treated };

    // 1. Scout
    let opp: Opportunity = match rs.call(&roles::scout_request(f, &row, dc)) {
        Err(e) => return finish(model_stop(r, "scout", e)),
        Ok(a) => match roles::parse_scout(&row, &a.content) {
            Ok(o) => o,
            Err(e) => return finish(blocked(r, "model_invalid", "scout", e)),
        },
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
    let mv = match rv.call(&roles::verifier_request(f, &opp, dc)) {
        Err(e) => return finish(model_stop(r, "verifier", e)),
        Ok(a) => match roles::parse_verifier(&a.content) {
            Ok(v) => v,
            Err(e) => return finish(blocked(r, "model_invalid", "verifier", e)),
        },
    };
    let (status, notes) = combine(&det, &mv);
    r.verification = Some(json!({"status": status, "deterministic": det_json, "model": {"verdict": mv.verdict, "checks": mv.checks.iter().map(|c| json!({"id": c.0, "result": c.1})).collect::<Vec<_>>(), "rationale": mv.rationale},
                                 "disagreements": notes, "human_review_required": status == "weakened"}));
    if status == "refuted" {
        return finish(blocked(r, "verifier_refuted", "verifier", mv.rationale));
    }

    // 4. Builder + byte-exact compile
    let req = match roles::builder_request(f, &opp, &row, catalog, dc) {
        Ok(q) => q,
        Err(e) => return finish(blocked(r, "precondition_missing", "builder", e)),
    };
    let proposal = match rb.call(&req) {
        Err(e) => return finish(model_stop(r, "builder", e)),
        Ok(a) => match roles::parse_builder(&a.content) {
            Ok(p) => p,
            Err(e) => return finish(blocked(r, "model_invalid", "builder", e)),
        },
    };
    match compile(catalog, f, &row, &opp, &proposal) {
        Err(d) => finish(blocked(r, &format!("compile_denied:{}", d.code), "compile", d.why)),
        Ok(c) => {
            r.rubric = (c.kind != "no_change").then(|| rubric::score(f, &row, &opp, &c, catalog));
            r.status = if c.kind == "no_change" { "no_change" } else { "proposed" }.into();
            r.reason = if c.kind == "no_change" { "builder_no_safe_change" } else { "compiled" }.into();
            r.stage = "compile".into();
            r.detail = c.rationale.chars().take(300).collect();
            r.compiled = Some(c.to_json());
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
    let _ = Role::Scout;
    Ok(json!({
        "contract": "reasoning-run/l2-0", "baseline": {"label": catalog.label, "source": catalog.source}, "sensor_semantics": report["semantics"], "data_source": source.as_str(),
        "quality_claims": "forbidden", "opt_in_derived_aggregates": opts.allow_derived_aggregates,
        "summary": {"corroborated": findings.len(), "proposed": n("proposed"), "no_change": n("no_change"), "unlinked": n("unlinked"), "blocked": n("blocked"), "skipped_not_corroborated": skipped.len()},
        "skipped": skipped.iter().map(|s| json!({"index": s.index, "metric": s.metric, "status": s.status, "reason": s.reason})).collect::<Vec<_>>(),
        "findings": results.iter().map(Reasoned::to_json).collect::<Vec<_>>(),
        "doubles": doubles,
    }))
}

pub fn row_of(f: &Finding) -> Option<Row> {
    map_finding(f)
}
