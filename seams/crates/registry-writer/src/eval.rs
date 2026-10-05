//! W11: one EVALUATION RUN on agent-core, as the Python proof (`scripts/regression/prove_fails_on_base.py`) does it, but through the
//! guarded writer: create a MANUAL-origin draft proposal (so the 10 per 24 h `auto_detect` quota is not consumed), put the candidate
//! changes plus the regression `eval_suite` in it, `validate`, `freeze`, `evaluate`. Never approve, publish, promote or reject: the
//! allow-list admits `freeze` and `evaluate` only.
//!
//! A run answers a raw JSON record (the contract of `scripts/regression/judge_story.py`):
//! `{label, proposal_id, verdict, gate_items[], per_case_native{case: {passed, reason}}, detail, problem, infra_retries[], attempts}`.
//! `verdict` is agent-core's own (`pass | fail | failed_infra`); `failed_infra` also stands for a step that could not complete
//! (`problem` names the step, the HTTP status and the registry's closed problem code, never free text).
//!
//! Retries are bounded and only for infrastructure: a transport failure, a 5xx or 429, or an `evaluate` that answered
//! `failed_infra` (the real JEV provider drops intermittently). A deterministic refusal (422 on the draft, validation violations,
//! 409 on freeze) is not retried. Every try opens a new throwaway draft with its own `Idempotency-Key`
//! (`<finding key>-<label>-t<n>`), so a replay of the same finding+label+try never opens a second one.
use crate::guard;
use crate::transport::Reply;
use crate::writer::{Writer, clip};
use serde_json::{Map, Value, json};
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct EvalOptions {
    /// Extra tries after the first one when the failure is infrastructure.
    pub infra_retries: u32,
    pub backoff: Duration,
    /// Injected so the offline tests do not wait.
    pub sleep: fn(Duration),
}

impl Default for EvalOptions {
    fn default() -> Self {
        EvalOptions { infra_retries: 4, backoff: Duration::from_secs(20), sleep: std::thread::sleep }
    }
}

/// What to evaluate: the suite and the candidate changes of ONE run (empty `changes` = the BASE).
pub struct EvalJob<'a> {
    pub label: &'a str,
    pub key: &'a str,
    pub agent_id: &'a str,
    pub suite: &'a Value,
    pub agent_entity: Option<&'a Value>,
    pub changes: &'a [Value],
}

/// Every gate or guardrail metric the agent declares needs a threshold: added with `noise_margin` 0 and no floor (a floor is a human
/// or measured decision). Same rule as `attach_eval_suite.with_default_thresholds`.
pub fn with_default_thresholds(suite: &Value, agent: Option<&Value>) -> Value {
    let mut out = suite.clone();
    let mut th: Map<String, Value> = suite["thresholds"].as_object().cloned().unwrap_or_default();
    for m in agent.and_then(|a| a["metrics"].as_array()).into_iter().flatten() {
        let (Some(id), Some(role)) = (m["id"].as_str(), m["role"].as_str()) else { continue };
        if matches!(role, "gate" | "guardrail") && !th.contains_key(id) {
            th.insert(id.into(), json!({"noise_margin": "0"}));
        }
    }
    out["thresholds"] = Value::Object(th);
    out
}

/// Per-scenario pass/fail from agent-core's EvalReport (`runs.cand_on_new.scenarios`, a scenario passes only if all its repetitions
/// pass) with the first failing repetition's reasons (clipped; they come from the scorer's closed failure messages).
pub fn native_from_report(rep: &Value, case_ids: &[String]) -> Value {
    let scen = rep["runs"]["cand_on_new"]["scenarios"].as_object();
    let mut reasons: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for r in rep["results"].as_array().into_iter().flatten() {
        let sc = &r["score"];
        if r["run"] == "cand_on_new" && sc["passed"].as_bool() == Some(false) {
            if let Some(id) = r["scenario_id"].as_str() {
                reasons.entry(id.into()).or_insert_with(|| sc["failures"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect());
            }
        }
    }
    let mut out = Map::new();
    for id in case_ids {
        if let Some(p) = scen.and_then(|s| s.get(id)) {
            let why = clip(&reasons.get(id).cloned().unwrap_or_default().join("; "), 400);
            out.insert(id.clone(), json!({"passed": p.as_bool() == Some(true), "reason": why}));
        }
    }
    Value::Object(out)
}

/// A closed record of a failed step: step name, http status, registry problem code (lower-case and underscores only) and whether a
/// retry can help.
struct Problem {
    step: &'static str,
    http: Option<u16>,
    code: String,
    retryable: bool,
}

impl Problem {
    fn of(step: &'static str, r: &Reply) -> Problem {
        let code = r.body["code"].as_str().filter(|c| c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')).unwrap_or("").to_string();
        Problem { step, http: Some(r.status), code, retryable: r.status >= 500 || r.status == 429 }
    }
    fn inherit_or(step: &'static str, r: &Reply, code: Option<&'static str>) -> Problem {
        let mut p = Problem::of(step, r);
        if let Some(c) = code {
            p.code = c.into();
            p.retryable = false;
        }
        p
    }
    fn to_json(&self) -> Value {
        json!({"step": self.step, "http": self.http, "code": self.code})
    }
}

/// INH1: closed codes of a Core that did not take `release_settings.inherit_from`, read from the status and the (never stored) message.
/// `None` when the draft does not carry a donor reference or the answer is not about it.
fn inherit_problem(job_changes: &[Value], r: &Reply, validate: bool) -> Option<&'static str> {
    let carries = job_changes.iter().any(|c| c["kind"] == "release_settings" && c["content"]["inherit_from"].is_string());
    if !carries {
        return None;
    }
    let text = r.body.to_string().to_lowercase();
    if text.contains("inherit_from") && text.contains("extra") {
        return Some("core_without_inherit_from"); // older Core: the field is an unknown (forbidden) one of ReleaseSettings
    }
    if text.contains("inherit_from") {
        return Some("inherit_from_rejected"); // e.g. the agent has a base release
    }
    (validate && r.status == 404).then_some("donor_release_unknown")
}

/// `(record, retryable)`.
type Try = (Value, bool);

fn failed(label: &str, pid: Option<&str>, p: &Problem) -> Try {
    (json!({"label": label, "proposal_id": pid, "verdict": "failed_infra", "problem": p.to_json(), "per_case_native": {}, "gate_items": [], "detail": null}), p.retryable)
}

impl Writer<'_> {
    fn eval_call(&self, step: &'static str, method: &str, path: String, idem: Option<&str>, body: Option<Value>) -> Result<Reply, Problem> {
        match self.call(method, path, self.registry_token(), idem, body) {
            Ok(r) => Ok(r),
            Err((reason, _)) => Err(Problem { step, http: None, code: reason.code().into(), retryable: matches!(reason, crate::Reason::RegistryUnreachable | crate::Reason::OutcomeUnknown) }),
        }
    }

    /// Content of a live entity (`kind`, `id`) through the guarded read route; `None` when it cannot be read.
    pub fn fetch_content(&self, kind: &str, id: &str) -> Option<Value> {
        let path = crate::baseline::entity_path(kind, id)?;
        let r = self.call("GET", path, self.registry_token(), None, None).ok()?;
        if !(200..300).contains(&r.status) {
            return None;
        }
        let b = r.body;
        Some(["content", "spec"].iter().find(|k| b[**k].is_object()).map_or(b.clone(), |k| b[*k].clone()))
    }

    /// `GET /v1/registry/aliases/{agent}/{alias}`: `None` when absent or unreadable.
    pub fn fetch_alias(&self, agent: &str, alias: &str) -> Option<Value> {
        let r = self.call("GET", format!("/v1/registry/aliases/{agent}/{alias}"), self.registry_token(), None, None).ok()?;
        (200..300).contains(&r.status).then_some(r.body)
    }

    /// One try, no retry.
    fn evaluate_once(&self, job: &EvalJob, tries: u32) -> Try {
        let suite = with_default_thresholds(job.suite, job.agent_entity);
        let (suite_id, suite_version) = (suite["id"].as_str().unwrap_or("").to_string(), suite["version"].as_str().unwrap_or("").to_string());
        if !guard::ok_seg(job.agent_id) || !guard::ok_seg(&suite_id) {
            return failed(job.label, None, &Problem { step: "input", http: None, code: "unsafe_id".into(), retryable: false });
        }
        let idem = format!("{}-{}-t{tries}", job.key, job.label);
        let title = clip(&format!("[improvement-engine] evaluation {} {suite_id}", job.label), 120);
        let r = match self.eval_call("create", "POST", "/v1/registry/proposals".into(), Some(&idem), Some(json!({"agent_id": job.agent_id, "origin": "manual", "title": title}))) {
            Ok(r) => r,
            Err(p) => return failed(job.label, None, &p),
        };
        if !(200..300).contains(&r.status) {
            return failed(job.label, None, &Problem::of("create", &r));
        }
        let (Some(pid), Some(rev)) = (r.body["proposal_id"].as_str().filter(|p| guard::ok_seg(p)).map(str::to_string), r.body["rev"].as_u64()) else {
            return failed(job.label, None, &Problem { step: "create", http: Some(r.status), code: "malformed_answer".into(), retryable: false });
        };
        let pid = pid.as_str();
        let base = format!("/v1/registry/proposals/{pid}");
        // agent-core wants description, rationale and changelog on every change (a 422 otherwise, seen live on the new-agent closure).
        let mut draft: Vec<Value> = job.changes.iter().cloned().map(|mut c| {
            if !c["docs"].is_object() {
                c["docs"] = json!({});
            }
            for (k, d) in [("description", "candidate patch"), ("rationale", "see proposal"), ("changelog", "patch")] {
                if c["docs"][k].as_str().is_none_or(str::is_empty) {
                    c["docs"][k] = json!(d);
                }
            }
            c
        }).collect();
        draft.push(json!({"kind": "eval_suite", "content": suite, "docs": {
            "description": clip(&format!("Synthetic regression eval_suite {suite_id} for {}", job.agent_id), 4000),
            "rationale": "Regression suite derived from a detected finding; must fail on the base and pass with the candidate.",
            "changelog": "Adds the suite (new yardstick)."}}));
        let step = |name: &'static str, method: &str, path: String, body: Option<Value>| self.eval_call(name, method, path, None, body);
        match step("put_draft", "PUT", format!("{base}/draft"), Some(json!({"expected_rev": rev, "changes": draft}))) {
            Ok(r) if (200..300).contains(&r.status) => {}
            Ok(r) => return failed(job.label, Some(pid), &Problem::inherit_or("put_draft", &r, inherit_problem(job.changes, &r, false))),
            Err(p) => return failed(job.label, Some(pid), &p),
        }
        match step("validate", "POST", format!("{base}/validate"), None) {
            Ok(r) if (200..300).contains(&r.status) => {
                if r.body["violations"].as_array().is_none_or(|v| !v.is_empty()) {
                    let code = inherit_problem(job.changes, &r, true).unwrap_or("violations");
                    return failed(job.label, Some(pid), &Problem { step: "validate", http: Some(r.status), code: code.into(), retryable: false });
                }
            }
            Ok(r) => return failed(job.label, Some(pid), &Problem::inherit_or("validate", &r, inherit_problem(job.changes, &r, true))),
            Err(p) => return failed(job.label, Some(pid), &p),
        }
        match step("freeze", "POST", format!("{base}/freeze"), None) {
            Ok(r) if (200..300).contains(&r.status) => {}
            Ok(r) => return failed(job.label, Some(pid), &Problem::of("freeze", &r)),
            Err(p) => return failed(job.label, Some(pid), &p),
        }
        let ev = match step("evaluate", "POST", format!("{base}/evaluate"), Some(json!({"suite_id": suite_id, "suite_version": suite_version}))) {
            Ok(r) => r,
            Err(p) => return failed(job.label, Some(pid), &p),
        };
        // 409 carries the report under `payload` (an evaluation that did not pass the gate).
        let rep = if ev.status == 409 && ev.body["payload"].is_object() { &ev.body["payload"] } else { &ev.body };
        let Some(verdict) = rep["verdict"].as_str().filter(|v| matches!(*v, "pass" | "fail" | "failed_infra")) else {
            return failed(job.label, Some(pid), &Problem::of("evaluate", &ev));
        };
        let case_ids: Vec<String> = suite["scenarios"].as_array().into_iter().flatten().filter_map(|s| s["id"].as_str().map(str::to_string)).collect();
        let items: Vec<Value> = rep["items"].as_array().into_iter().flatten().map(|i| json!({"metric": i["metric_id"], "phase": i["phase"], "passed": i["passed"], "value": i["value"], "reason": i["reason"].as_str().filter(|r| !r.is_empty()).map(|r| clip(r, 200))})).collect();
        let detail = rep["detail"].as_str().map(|d| clip(d, 200));
        let run = json!({"label": job.label, "proposal_id": pid, "verdict": verdict, "gate_items": items, "per_case_native": native_from_report(rep, &case_ids), "detail": detail, "problem": null});
        (run, verdict == "failed_infra")
    }

    /// One evaluation with bounded infrastructure retries. The returned record lists every retry (`infra_retries`).
    pub fn evaluate_run(&self, job: &EvalJob, opts: &EvalOptions) -> Value {
        let mut retries: Vec<Value> = vec![];
        let mut run = Value::Null;
        for n in 0..=opts.infra_retries {
            let (r, retryable) = self.evaluate_once(job, n);
            run = r;
            if !retryable || n == opts.infra_retries {
                break;
            }
            retries.push(json!({"proposal_id": run["proposal_id"], "problem": run["problem"], "detail": run["detail"]}));
            (opts.sleep)(opts.backoff);
        }
        run["infra_retries"] = Value::Array(retries);
        run
    }
}
