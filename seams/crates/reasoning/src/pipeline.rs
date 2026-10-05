//! Scout -> Verifier -> Builder over one finding, on three `ModelPort`s.
//!
//! Stops are typed and final for the finding: `unlinked` (no mapping row), `blocked(<reason>)` (a port refused or failed, an answer
//! was invalid, the deterministic recompute refuted the claim, the independent verifier refuted it, the compiler denied the
//! proposal). There is no fallback to another port and no retry with a different prompt. Every model call is recorded under the
//! label and model id of the port that handled it; `doubles[]` lists every call that was not an answered gateway call.
use crate::catalog::Catalog;
use crate::finding::{Finding, Source, deterministic_checks};
use crate::mapping::{Caps, Mapped, Row, Table};
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
    engine::models::record::tier_of(model_id)
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

/// Transient gateway failures (HTTP 5xx, 429, timeouts and I/O errors, unreachable) are retried before the role stops as
/// `model_unavailable`: at most this many calls in total, with jittered exponential backoff, and never beyond the cost budget below.
pub const TRANSIENT_ATTEMPTS: u32 = 3;
/// USD that the calls of one role may have cost already for a transient retry to still start (the per-attempt budget of a finding).
pub const TRANSIENT_COST_BUDGET_USD: f64 = 0.50;

static TRANSIENT_BACKOFF_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(400);

/// Base of the transient backoff (tests set it to 1 ms).
pub fn set_transient_backoff_ms(ms: u64) {
    TRANSIENT_BACKOFF_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
}

thread_local! {
    /// Transient retries made by the roles of the attempt in progress (reported in `metering.transient_retries`).
    static TRANSIENT_RETRIES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Whether the `Unavailable` reason is worth another try: a gateway 5xx/429, a timeout or I/O error, an unreachable endpoint.
pub fn is_transient(why: &str) -> bool {
    let Some(code) = why.strip_prefix("gateway_http_") else {
        return why.starts_with("gateway_io") || why.starts_with("gateway_unreachable");
    };
    let status = code.split(':').next().unwrap_or("");
    status == "429" || status.starts_with('5')
}

fn transient_pause(retry: u32) {
    let base = TRANSIENT_BACKOFF_MS.load(std::sync::atomic::Ordering::Relaxed);
    let exp = base.saturating_mul(1u64 << retry.min(6));
    let jitter = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::from(d.subsec_nanos())) % (exp / 2 + 1);
    std::thread::sleep(std::time::Duration::from_millis(exp + jitter));
}

/// One role call with up to `MAX_RETRIES` retries. A retry is the SAME request plus the previous problem as `feedback`. A transient
/// gateway failure is retried separately (`TRANSIENT_ATTEMPTS` calls in total, backoff, cost budget) and does not use the answer retries.
/// Returns how many calls were made.
fn ask<T>(port: &Recording, req: &ModelRequest, stage: &'static str, base_attempt: usize, check: &dyn Fn(&Value) -> Result<T, Reject>) -> (Asked<T>, usize) {
    let mut last: Option<Reject> = None;
    let (mut attempt, mut calls, mut transient) = (0usize, 0usize, 0u32);
    while attempt <= MAX_RETRIES {
        let mut r = req.clone();
        if let Some((_, _, why)) = &last {
            r.payload["feedback"] = json!(format!("Attempt {attempt} was rejected: {}. Answer again with ONE JSON object that follows output_schema exactly.", feedback_text(why)));
        }
        // the story span of this call: gateway calls made from here carry `traceparent` with the stage span as parent
        calls += 1;
        engine::trace::set_stage(stage, u32::try_from(base_attempt + calls).unwrap_or(u32::MAX));
        match port.call(&r) {
            Err(ModelError::Invalid(w)) => last = Some((stage, "model_invalid".into(), packaging_hint(&w))),
            Err(ModelError::Unavailable(w)) if is_transient(&w) && transient + 1 < TRANSIENT_ATTEMPTS && port.calls().iter().filter_map(|c| c.usage.as_ref()).map(|u| u.cost_f64()).sum::<f64>() < TRANSIENT_COST_BUDGET_USD => {
                transient += 1;
                TRANSIENT_RETRIES.with(|t| t.set(t.get() + 1));
                transient_pause(transient);
                continue;
            }
            Err(e) => return (Asked::Stopped(e), calls),
            Ok(a) => match check(&a.content) {
                Ok(t) => return (Asked::Done(t), calls),
                Err(rej) => last = Some(rej),
            },
        }
        attempt += 1;
    }
    (Asked::Rejected(last.expect("at least one attempt")), calls)
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
    /// One `pulso.model_call/1` per call (content, tokens, USD from the price table, ids): kept OUT of `to_json` (the report and the
    /// job record carry no model free text); the value loop persists them apart.
    pub call_records: Vec<Value>,
    pub doubles: Vec<Value>,
    pub independence: Value,
    pub source: &'static str,
    /// Decision dossier (W1-2) of a proposed finding, built without a regression verdict: `announce` stays false until the REG1
    /// verdict story is supplied (`reason_cli dossier --verdict`), the dossier says so. `{"error": ..}` if its PII guard refused.
    pub dossier: Option<Value>,
    /// Cost, tokens, latency and attempts of every model call of this finding, and the Builder tier that answered.
    pub metering: Value,
    /// MAP1: the candidate this attempt tried (`rank`, `target_ref`, `justification`, `evidence`, `candidates_total`) and the honest label of the mapping.
    pub candidate: Option<Value>,
    /// MAP1: set when a person owns the finding (`owner`, `evidence`, `note {es, pt}`); no model was called.
    pub human_owned: Option<Value>,
}

impl Reasoned {
    pub fn to_json(&self) -> Value {
        json!({"finding_id": self.finding_id, "status": self.status, "reason": self.reason, "stage": self.stage, "detail": self.detail, "mapping_row": self.mapping_row,
               "source": self.source, "opportunity": self.opportunity, "verification": self.verification, "proposal": self.compiled, "rubric": self.rubric,
               "model_calls": self.calls, "doubles": self.doubles, "independence": self.independence, "dossier": self.dossier, "metering": self.metering,
               "candidate": self.candidate, "human_owned": self.human_owned})
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

/// The whole row: the Scout chooses among every target of the mapping row (kept for callers that do not iterate candidates).
pub fn reason(catalog: &Catalog, f: &Finding, ports: &Ports, opts: &Opts) -> Reasoned {
    reason_candidate(catalog, f, ports, opts, None)
}

/// The label every candidate attempt carries: a mapping is a hypothesis of where to intervene, never a cause.
pub fn mapping_label(row: &Row, t: &crate::mapping::Target) -> Value {
    let table = Table::bundled();
    json!({"claim": "hypothesis_of_where_to_intervene_not_a_cause", "notice": {"en": table.notice_en, "es": table.notice_es, "pt": table.notice_pt}, "row": row.id, "topic": row.topic,
           "rank": t.rank, "candidates_total": row.candidates_total, "target_ref": t.target_ref, "agent": t.agent, "justification": t.justification, "evidence": t.evidence,
           "proof_support": t.proof_support, "announceable_now": t.announceable_now})
}

/// One attempt of the finding. `only = Some(target_ref)` restricts the mapping row to that ONE candidate (the loop tries candidates in
/// rank order); `None` leaves the whole row to the Scout.
pub fn reason_candidate(catalog: &Catalog, f: &Finding, ports: &Ports, opts: &Opts, only: Option<&str>) -> Reasoned {
    let (rs, rv, rb) = (Rc::new(Recording::new(ports.scout.clone())), Rc::new(Recording::new(ports.verifier.clone())), Rc::new(Recording::new(ports.builder.clone())));
    let rbe = ports.builder_escalation.as_ref().map(|p| Rc::new(Recording::new(p.clone())));
    let mut r = Reasoned {
        finding_id: f.id.clone(), status: "blocked".into(), reason: String::new(), stage: String::new(), detail: String::new(), mapping_row: None, opportunity: None, verification: None,
        compiled: None, compiled_raw: None, rubric: None, calls: vec![], call_records: vec![], doubles: vec![], source: f.source.as_str(), dossier: None, metering: Value::Null, candidate: None, human_owned: None,
        independence: json!({"scout_model": ports.scout.model_id(), "verifier_model": ports.verifier.model_id(), "builder_model": ports.builder.model_id(),
                             "builder_escalation_model": ports.builder_escalation.as_ref().map(|p| p.model_id()),
                             "verifier_separate_port": !Rc::ptr_eq(&ports.scout, &ports.verifier),
                             "level": independence_level(&ports.scout.model_id(), &ports.verifier.model_id())}),
    };
    TRANSIENT_RETRIES.with(|t| t.set(0));
    let attempts = std::cell::RefCell::new(json!({"scout": 0, "verifier": 0, "builder": 0, "builder_escalation": 0}));
    let tier_used: std::cell::RefCell<Option<(String, String, bool)>> = std::cell::RefCell::new(None);
    let finish = |mut r: Reasoned| {
        let all: Vec<&Rc<Recording>> = [Some(&rs), Some(&rv), Some(&rb), rbe.as_ref()].into_iter().flatten().collect();
        r.calls = all.iter().flat_map(|x| x.calls().iter().map(|c| c.to_json()).collect::<Vec<_>>()).collect();
        r.doubles = all.iter().flat_map(|x| x.doubles()).collect();
        let recs: Vec<engine::models::CallRecord> = all.iter().flat_map(|x| x.calls()).collect();
        let story = engine::trace::current();
        let mut per_role: std::collections::HashMap<&'static str, u32> = std::collections::HashMap::new();
        r.call_records = recs
            .iter()
            .map(|c| {
                let n = per_role.entry(c.role.as_str()).or_insert(0);
                *n += 1;
                let mut v = engine::models::record::call_record(c, *n, story.as_ref());
                if v["evidence_ref"].is_null() {
                    v["evidence_ref"] = json!(f.evidence_ref());
                }
                v
            })
            .collect();
        let usage = |f: &dyn Fn(&engine::models::Usage) -> f64| recs.iter().filter_map(|c| c.usage.as_ref()).map(f).sum::<f64>();
        let tu = tier_used.borrow().clone();
        r.metering = json!({
            "calls": recs.len(), "attempts": attempts.borrow().clone(), "transient_retries": TRANSIENT_RETRIES.with(std::cell::Cell::get),
            "tokens_in": usage(&|u| u.tokens_in as f64) as u64, "tokens_out": usage(&|u| u.tokens_out as f64) as u64,
            "cost_usd": (usage(&|u| u.cost_f64()) * 1e6).round() / 1e6,
            "latency_ms": recs.iter().map(|c| c.wall_ms).sum::<u64>(),
            "builder": tu.map(|(tier, model, escalated)| json!({"tier": tier, "model": model, "escalated": escalated})),
        });
        if r.status == "proposed" {
            let real = [&ports.scout, &ports.verifier, &ports.builder].iter().all(|p| p.label() == Label::Gateway);
            let labels = crate::dossier::Labels { runtime: if real { crate::dossier::Runtime::Real } else { crate::dossier::Runtime::Doubles }, ..Default::default() };
            let record = json!({"proposal": r.compiled, "doubles": r.doubles, "rubric": r.rubric});
            r.dossier = Some(crate::dossier::build(&f.to_signal_json(), &record, None, &labels).unwrap_or_else(|e| json!({"error": e})));
        }
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
    let row = match Table::bundled().classify(f) {
        Mapped::HumanOwned(h) => {
            // A person owns this finding: no model is called and no proposal exists.
            r.status = "human_owned".into();
            r.reason = h.id.into();
            r.stage = "mapping".into();
            r.detail = h.note_es.chars().take(300).collect();
            r.human_owned = Some(json!({"id": h.id, "owner": h.owner, "evidence": h.evidence, "note": {"es": h.note_es, "pt": h.note_pt}, "builder_proposal": false}));
            if h.policy.is_object() {
                // ART2: a policy finding is a HYPOTHESIS for a person (boundary evidence, no draft chosen by a model). When the table carries a
                // structured `tighten_to`, a tighten-only draft is compiled DETERMINISTICALLY (no model call) and waits for the owner.
                let pid = h.policy["policy"].as_str().unwrap_or("");
                let pol = catalog.policy(pid).cloned().unwrap_or_else(|| json!({"id": pid, "owner": h.owner}));
                let hyp = crate::art2::policy_hypothesis(&pol, h.policy["registry_threshold"].as_f64().unwrap_or(0.0), h.policy["document_threshold"].as_f64().unwrap_or(0.0));
                r.status = "policy_hypothesis".into();
                r.reason = "policy_hypothesis".into();
                r.stage = "mapping".into();
                if let Some(ho) = r.human_owned.as_mut() {
                    ho["policy_hypothesis"] = hyp;
                }
                if h.policy["tighten_to"].is_number() {
                    match crate::patch::compile_policy_tighten(catalog, &h.policy) {
                        Ok(c) => {
                            r.status = "needs_owner_ack".into();
                            r.reason = "needs_owner_ack".into();
                            r.stage = "compile".into();
                            r.compiled = Some(c.to_json());
                            r.compiled_raw = Some(c);
                        }
                        Err(d) => r.detail = format!("tighten draft refused: {}: {}", d.code, d.why).chars().take(300).collect(),
                    }
                }
            }
            return finish(r);
        }
        Mapped::Unmapped => {
            let (code, why) = crate::mapping::unlinked_reason(f);
            r.status = "unlinked".into();
            r.reason = code.into();
            r.stage = "mapping".into();
            r.detail = why;
            return finish(r);
        }
        Mapped::Row(row) => match only {
            None => row,
            Some(t) => match row.only(t) {
                Some(one) => one,
                None => return finish(blocked(r, "precondition_missing", "mapping", format!("{t} is not a candidate of the mapping row"))),
            },
        },
    };
    r.mapping_row = Some(row.id.into());
    if let Some(t) = row.targets.first().filter(|_| only.is_some()) {
        r.candidate = Some(mapping_label(&row, t));
    }
    let hosted = [&ports.scout, &ports.verifier, &ports.builder].iter().any(|p| p.label() == Label::Gateway);
    if f.source.derived() && hosted && !opts.allow_derived_aggregates {
        return finish(blocked(r, "derived_data_opt_in_required", "policy", format!("{} aggregates reach a real hosted model only with the explicit opt-in flag", f.source.as_str())));
    }
    // After the opt-in (or for synthetic data) the payload is a treated aggregate: the scanner of each port is the gate.
    let dc = if f.source == Source::Synthetic { DataClass::Synthetic } else { DataClass::Treated };

    // 1. Scout (bounded retries with the parse problem fed back)
    let (got, n) = ask(&rs, &roles::scout_request(f, &row, dc), "scout", 0, &|a| roles::parse_scout(&row, a).map_err(|e| ("scout", "model_invalid".to_string(), e)));
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
    let (got, n) = ask(&rv, &roles::verifier_request(f, &opp, dc), "verifier", 0, &|a| roles::parse_verifier(a).map_err(|e| ("verifier", "model_invalid".to_string(), e)));
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
    let (got, n) = ask(&rb, &req, "builder", 0, &check);
    attempts.borrow_mut()["builder"] = json!(n);
    *tier_used.borrow_mut() = Some((tier_of(&ports.builder.model_id()).into(), ports.builder.model_id(), false));
    let got = match (got, &rbe) {
        (Asked::Rejected(_), Some(esc)) => {
            let (g2, n2) = ask(esc, &req, "builder", n, &check);
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
            // ART3 / FLOW1: an independent Verifier call reviews a compiled tool link or flow edit on structured facts (read-only link, valid edge;
            // protected nodes untouched, pass-through, exits preserved). The compiler already enforced all of it; the model can only refute, never widen.
            let review = match c.kind.as_str() {
                "link_tool" => roles::link_review_request(&c, dc).map(|q| ("link_review", "link_review_refuted", &roles::LINK_CHECK_IDS[..], q)),
                "flow_edit" => roles::flow_review_request(&c, dc).map(|q| ("flow_review", "flow_review_refuted", &roles::FLOW_CHECK_IDS[..], q)),
                _ => None,
            };
            if let Some((stage, refuted, ids, rq)) = review {
                let (rg, n) = ask(&rv, &rq, stage, 0, &|a| roles::parse_review(a, ids).map_err(|e| (stage, "model_invalid".to_string(), e)));
                attempts.borrow_mut()[stage] = json!(n);
                match rg {
                    Asked::Done((ok, why)) => {
                        r.verification = Some(json!({"status": r.verification.as_ref().map_or(Value::Null, |v| v["status"].clone()), "claim_verification": r.verification.clone(), stage: {"supported": ok, "rationale": why}}));
                        if !ok {
                            return finish(blocked(r, refuted, stage, why));
                        }
                    }
                    Asked::Stopped(e) => return finish(model_stop(r, stage, e)),
                    Asked::Rejected((st, code, why)) => return finish(blocked(r, &code, st, why)),
                }
            }
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
        "summary": {"corroborated": findings.len(), "proposed": n("proposed"), "no_change": n("no_change"), "unlinked": n("unlinked"), "human_owned": n("human_owned"), "blocked": n("blocked"), "skipped_not_corroborated": skipped.len(),
                    // an unlinked finding is descriptive (explicit reason), never a failure: only `blocked` counts against the roles
                    "unlinked_by_reason": by_reason, "failed": n("blocked"), "linked": findings.len() - n("unlinked") - n("no_change") - n("human_owned"),
                    "cost_usd": (results.iter().map(|r| r.metering["cost_usd"].as_f64().unwrap_or(0.0)).sum::<f64>() * 1e6).round() / 1e6},
        "skipped": skipped.iter().map(|s| json!({"index": s.index, "metric": s.metric, "status": s.status, "reason": s.reason})).collect::<Vec<_>>(),
        "findings": results.iter().map(Reasoned::to_json).collect::<Vec<_>>(),
        "doubles": doubles,
    }))
}

pub fn row_of(f: &Finding) -> Option<Row> {
    crate::mapping::map_finding(f)
}

/// The candidate targets of a finding in the order the loop tries them (at most `MAX_CANDIDATES`), or empty when the finding has no
/// row (unlinked, human owned, direction not up).
pub fn candidate_plan(f: &Finding, caps: Caps) -> Vec<String> {
    if f.direction != "up" {
        return vec![];
    }
    row_of(f).map(|r| r.ordered(caps).iter().map(|t| t.target_ref.clone()).collect()).unwrap_or_default()
}
