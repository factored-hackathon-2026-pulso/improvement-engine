//! /internal/v1/automation: the read model behind the platform's "Automatizacion" screens (case-type maturity) plus the
//! team thresholds and a SIMULATED approve / publish-to-staging path. Keyed on the agent-core proposal id and its target
//! artifact (`agent_id@alias`); nothing here is console specific. Metrics come from the pure `maturity` crate; every
//! number carries its source and a `simulated` flag, and what cannot be computed says so.
use crate::app::{App, Req, Resp, bearer_ok, ok, problem, resp};
use crate::event::{NewEvent, RunEventSink};
use maturity::{Disposition, InputSource, JsonSource, Maturity, Metric, Stage, Thresholds};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const AUTOMATION_PREFIX: &str = "/internal/v1/automation";
const AUDIT_RUN: &str = "automation-audit";
const STAGE_KEYS: [&str; 4] = ["1", "2", "3", "agent"];

pub struct Automation {
    e0: Option<JsonSource>,
    sim: Option<JsonSource>,
    as_of: String,
    proposals: Value,
    state: Mutex<State>,
    admitter: Option<Arc<dyn TriggerAdmitter>>,
    triggers: Mutex<Triggers>,
}

/// Where an accepted trigger becomes an engine job. Same contract as `pg::repo::JobRepository::admit_keyed` (idempotent on
/// `key` per tenant: the same key returns the same job id and queues nothing), so the runner wires its repository in one line.
/// debug-api does not depend on the job store; the embedding binary injects it.
pub trait TriggerAdmitter: Send + Sync {
    fn admit_keyed(&self, tenant: &str, key: &str) -> Result<String, String>;
}

/// In-process admitter (keyed, idempotent): the default for the standalone binary and for tests.
#[derive(Default)]
pub struct MemoryAdmitter {
    inner: Mutex<(Vec<(String, String)>, bool)>,
}

impl MemoryAdmitter {
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn keys(&self) -> Vec<String> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).0.iter().map(|(k, _)| k.split_once('/').map_or(k.clone(), |x| x.1.to_string())).collect()
    }
    /// The next `admit_keyed` fails once (a store outage).
    pub fn fail_next(&self) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).1 = true;
    }
}

impl TriggerAdmitter for MemoryAdmitter {
    fn admit_keyed(&self, tenant: &str, key: &str) -> Result<String, String> {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if std::mem::take(&mut g.1) {
            return Err("job store unavailable".into());
        }
        let scoped = format!("{tenant}/{key}");
        if let Some((_, id)) = g.0.iter().find(|(k, _)| *k == scoped) {
            return Ok(id.clone());
        }
        let id = format!("job-{}", g.0.len());
        g.0.push((scoped, id.clone()));
        Ok(id)
    }
}

#[derive(Default)]
struct Triggers {
    order: Vec<String>,
    by_key: HashMap<String, TriggerRec>,
}

struct TriggerRec {
    digest: String,
    response: Value,
    view: Value,
}

struct State {
    thresholds: Thresholds,
    revision: u64,
    proposal_state: HashMap<String, &'static str>,
}

impl Automation {
    /// `e0`: treated aggregates (real); `sim`: the simulator's SIM-ONLY draft stream; `proposals`: `{type_id: {...}}`.
    pub fn from_json(e0: Option<&Value>, sim: Option<&Value>, proposals: Option<&Value>) -> Result<Automation, String> {
        let e0 = e0.map(|v| JsonSource::from_json(maturity::Source::E0Treated, false, v)).transpose().map_err(|e| format!("e0: {e}"))?;
        let simsrc = sim.map(|v| JsonSource::from_json(maturity::Source::SimDraftStream, true, v)).transpose().map_err(|e| format!("sim: {e}"))?;
        let as_of = sim.and_then(|v| v["as_of"].as_str()).unwrap_or("unknown").to_string();
        Ok(Automation {
            e0,
            sim: simsrc,
            as_of,
            proposals: proposals.cloned().unwrap_or_else(|| json!({})),
            state: Mutex::new(State { thresholds: Thresholds::default(), revision: 0, proposal_state: HashMap::new() }),
            admitter: None,
            triggers: Mutex::new(Triggers::default()),
        })
    }

    /// Enables `POST /triggers`: accepted triggers are admitted as keyed jobs through `admitter`.
    pub fn with_admitter(mut self, admitter: Arc<dyn TriggerAdmitter>) -> Automation {
        self.admitter = Some(admitter);
        self
    }

    fn trigger_list(&self) -> Value {
        let t = self.triggers.lock().unwrap_or_else(|p| p.into_inner());
        let items: Vec<Value> = t.order.iter().filter_map(|k| t.by_key.get(k).map(|r| r.view.clone())).collect();
        json!({"count": items.len(), "triggers": items})
    }

    fn trigger_summary(&self) -> Value {
        let t = self.triggers.lock().unwrap_or_else(|p| p.into_inner());
        let latest: Vec<Value> = t.order.iter().rev().take(5).filter_map(|k| t.by_key.get(k).map(|r| r.view.clone())).collect();
        json!({"count": t.order.len(), "latest": latest})
    }

    fn sources(&self) -> Vec<&dyn InputSource> {
        let mut v: Vec<&dyn InputSource> = Vec::new();
        if let Some(s) = &self.e0 {
            v.push(s);
        }
        if let Some(s) = &self.sim {
            v.push(s);
        }
        v
    }

    fn type_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for s in [self.sim.as_ref(), self.e0.as_ref()].into_iter().flatten() {
            for t in s.type_ids() {
                if !ids.contains(&t) {
                    ids.push(t);
                }
            }
        }
        ids
    }

    fn row(&self, id: &str, key: &str) -> Value {
        [self.sim.as_ref(), self.e0.as_ref()].into_iter().flatten().find_map(|s| s.row(id).and_then(|r| r.get(key)).cloned()).unwrap_or(Value::Null)
    }

    fn origin(&self) -> &'static str {
        match (&self.e0, &self.sim) {
            (Some(_), Some(_)) => "mixed",
            (None, Some(_)) => "simulated",
            (Some(_), None) => "e0_treated",
            _ => "none",
        }
    }

    /// Stated assumptions behind real (E0) numbers; empty when E0 is not a source.
    fn assumptions(&self) -> Value {
        if self.e0.is_some() { json!(["e0_disputar_cargo_as_cobro_indebido"]) } else { json!([]) }
    }

    fn doubles(&self) -> Value {
        if self.sim.is_some() {
            json!([{"id": "sim_draft_stream", "mode": "simulated", "scope": ["draft_dispositions", "case_types", "stage_history", "cases_today", "agent_runs", "approval", "publish_staging"]}])
        } else {
            json!([])
        }
    }

    fn proposal_view(&self, id: &str, st: &State) -> Value {
        match self.proposals.get(id) {
            None => Value::Null,
            Some(p) => {
                let mut v = p.clone();
                v["state"] = json!(st.proposal_state.get(id).copied().unwrap_or("proposed"));
                v["links"] = json!({"run": p["run_id"], "proposal_diff": p["proposal_id"]});
                v
            }
        }
    }

    fn measure(&self, id: &str, m: &Maturity) -> Value {
        let val = |kind: &str, x: &Metric| match x {
            Metric::Value { numerator, denominator, source, simulated, .. } => Some(json!({"kind": kind, "numerator": numerator, "denominator": denominator, "source": source.as_str(), "simulated": simulated})),
            _ => None,
        };
        let pick = match m.stage {
            Stage::Agent => val("agent_resolved", &m.agent_resolved),
            Stage::S3 => val("draft_accept_100", &m.draft_accept_100).or_else(|| val("tool_use_rate", &m.tool_use_rate)),
            Stage::S2 => val("tool_use_rate", &m.tool_use_rate).or_else(|| val("repeat_q", &m.repeat_q)),
            Stage::S1 => {
                let (n, d) = (self.row(id, "copilot_cases").as_u64(), self.row(id, "cases_total").as_u64());
                match (n, d, &self.sim) {
                    (Some(n), Some(d), Some(_)) => Some(json!({"kind": "copilot_use", "numerator": n, "denominator": d, "source": "sim_draft_stream", "simulated": true})),
                    _ => val("repeat_q", &m.repeat_q),
                }
            }
            Stage::S0 => None,
        };
        let mut out = pick.unwrap_or_else(|| json!({"kind": "none"}));
        if m.stage == Stage::Agent {
            out["handed"] = self.row(id, "agent_handed");
        }
        out
    }

    fn item(&self, id: &str, t: &Thresholds) -> Value {
        let m = maturity::evaluate(id, t, &self.sources());
        let mut j = m.to_json(t);
        j["label"] = self.row(id, "label");
        j["group"] = self.row(id, "group");
        j["measure"] = self.measure(id, &m);
        j
    }

    fn list(&self) -> Value {
        let st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let items: Vec<Value> = self.type_ids().iter().map(|id| self.item(id, &st.thresholds)).collect();
        let banner = items.iter().find(|c| c["agent_proposed"] == true).map(|c| {
            let id = c["type_id"].as_str().unwrap_or_default();
            let ps = self.proposal_view(id, &st);
            json!({"type_id": id, "label": c["label"], "proposal_id": ps["proposal_id"], "simulated": c["simulated"], "numerator": c["metrics"]["draft_accept_100"]["numerator"], "denominator": c["metrics"]["draft_accept_100"]["denominator"]})
        });
        json!({"as_of": self.as_of, "data_origin": self.origin(), "doubles": self.doubles(), "thresholds": maturity::thresholds_to_json(&st.thresholds), "revision": st.revision, "banner": banner, "assumptions": self.assumptions(), "triggers": self.trigger_summary(), "case_types": items})
    }

    fn detail(&self, id: &str) -> Option<Value> {
        if !self.type_ids().iter().any(|t| t == id) {
            return None;
        }
        let st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut j = self.item(id, &st.thresholds);
        let reached = match j["stage"].as_str() {
            Some("agent") => 4,
            _ => j["stage"].as_u64().unwrap_or(0) as usize,
        };
        let since = self.row(id, "stage_since");
        let mut history: Vec<Value> = STAGE_KEYS.iter().enumerate().take(reached.min(4)).filter(|(i, k)| *i < 3 || since.get(**k).is_some()).map(|(_, k)| json!({"stage": k, "since": since.get(*k).cloned().unwrap_or(Value::Null)})).collect();
        if j["agent_proposed"] == true {
            history.push(json!({"stage": "agent_proposed", "since": self.as_of}));
        }
        j["history"] = json!(history);
        let window = st.thresholds.draft_window as usize;
        let drafts = self.sources().into_iter().find_map(|s| s.inputs(id).and_then(|i| i.drafts).filter(|d| d.len() >= window).map(|d| (d, s.source(), s.simulated())));
        j["drafts_last_100"] = match drafts {
            Some((d, src, sim)) => {
                let last = &d[d.len() - window..];
                let n = |x: Disposition| last.iter().filter(|y| **y == x).count();
                json!({"status": "ok", "window": window, "as_is": n(Disposition::AsIs), "minor": n(Disposition::Minor), "discarded": n(Disposition::Discarded), "source": src.as_str(), "simulated": sim})
            }
            None => json!({"status": "not_computable", "reason": j["metrics"]["draft_accept_100"]["reason"]}),
        };
        let th = j["thresholds"].clone();
        let today = |m: &str| j["metrics"][m].clone();
        let row = |key: &str, metric: &str| {
            let min = th[key]["min"].as_f64().unwrap_or(0.0);
            let t = today(metric);
            let met = t["status"] == "ok" && t["value"].as_f64().is_some_and(|v| v >= min);
            let mut r = th[key].clone();
            r["key"] = json!(key);
            r["today"] = t;
            r["met"] = json!(met);
            r
        };
        j["thresholds_today"] = json!([row("stage1_to_2", "repeat_q"), row("stage2_to_3", "tool_use_rate"), row("stage3_to_agent", "draft_accept_100")]);
        j["proposal"] = self.proposal_view(id, &st);
        Some(j)
    }
}

fn is_admin(app: &App, r: &Req) -> Option<Resp> {
    let Some(token) = &app.cfg.admin_token else { return Some(problem("not_found", 404, json!({}))) };
    if bearer_ok(r, token) { None } else { Some(problem("unauthorized", 401, json!({}))) }
}

impl App {
    fn audit(&self, kind: &str, entity: &str, data: Value) {
        if self.store.head(AUDIT_RUN).is_none() {
            let _ = self.store.emit(AUDIT_RUN, NewEvent::new("run_started", "run", AUDIT_RUN, json!({"title": "Auditoria de automatizacion", "state": "completed", "origin": "system"})));
        }
        let _ = self.store.emit(AUDIT_RUN, NewEvent::new(kind, "automation", entity, data));
    }

    pub(crate) fn automation_route(&self, r: &Req) -> Resp {
        let Some(auto) = self.cfg.automation.as_ref() else { return problem("not_found", 404, json!({})) };
        let (m, rest) = (r.method.as_str(), r.path.strip_prefix(AUTOMATION_PREFIX).unwrap_or("").trim_start_matches('/'));
        let parts: Vec<&str> = rest.split('/').collect();
        if (m, parts.as_slice()) == ("PUT", &["config"][..]) {
            if let Some(e) = is_admin(self, r) {
                return e;
            }
            return self.put_config(auto, r);
        }
        if let Some(t) = &self.cfg.token {
            if !bearer_ok(r, t) {
                return problem("unauthorized", 401, json!({}));
            }
        }
        match (m, parts.as_slice()) {
            ("GET", ["triggers"]) => ok(&auto.trigger_list()),
            ("POST", ["triggers"]) => self.post_trigger(auto, r),
            ("GET", ["case-types"]) => ok(&auto.list()),
            ("GET", ["case-types", id]) => auto.detail(id).map_or_else(|| problem("not_found", 404, json!({})), |d| ok(&d)),
            ("POST", ["case-types", id, "proposal", action @ ("approve" | "publish-staging")]) => self.proposal_action(auto, id, action, r),
            _ => problem("not_found", 404, json!({})),
        }
    }

    fn put_config(&self, auto: &Automation, r: &Req) -> Resp {
        let Ok(body) = serde_json::from_slice::<Value>(&r.body) else { return problem("validation_error", 422, json!({})) };
        let mut st = auto.state.lock().unwrap_or_else(|p| p.into_inner());
        match maturity::thresholds_from_json(&st.thresholds, &body) {
            Err(e) => problem("validation_error", 422, json!({"message": e})),
            Ok(t) => {
                let before = maturity::thresholds_to_json(&st.thresholds);
                st.thresholds = t;
                st.revision += 1;
                let out = json!({"thresholds": maturity::thresholds_to_json(&st.thresholds), "revision": st.revision});
                self.audit("automation_config_changed", "thresholds", json!({"before": before, "after": out["thresholds"], "revision": st.revision, "actor": "local-admin"}));
                ok(&out)
            }
        }
    }

    /// SIMULATED: records a decision / publish request; nothing reaches agent-core or the platform.
    fn proposal_action(&self, auto: &Automation, id: &str, action: &str, r: &Req) -> Resp {
        if r.headers.get("x-csrf-token") != Some(&self.csrf) {
            return problem("csrf_failed", 403, json!({}));
        }
        let Some(p) = auto.proposals.get(id) else { return problem("not_found", 404, json!({})) };
        let body: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        let mut st = auto.state.lock().unwrap_or_else(|p| p.into_inner());
        let cur = st.proposal_state.get(id).copied().unwrap_or("proposed");
        let (want, next, kind) = match action {
            "approve" => ("proposed", "approved_simulated", "automation_proposal_approved_simulated"),
            _ => ("approved_simulated", "staged_simulated", "automation_publish_staging_simulated"),
        };
        if cur != want {
            return problem(if action == "approve" { "already_decided" } else { "approval_required" }, 409, json!({}));
        }
        if action == "approve" && body["candidate_hash"] != p["candidate_hash"] {
            return problem("candidate_hash_mismatch", 409, json!({}));
        }
        st.proposal_state.insert(id.to_string(), next);
        self.audit(kind, id, json!({"simulated": true, "proposal_id": p["proposal_id"], "target": p["target"], "candidate_hash": p["candidate_hash"], "step_up": "simulated"}));
        resp(200, &json!({"state": next, "simulated": true, "proposal_id": p["proposal_id"]}), vec![])
    }
}

const TRIGGER_SCHEMA: &str = "pulso.trigger.v1";
const TRIGGER_JOB_PREFIX: &str = "trigger:";
/// (kind, event type) pairs the poller produces; anything else is `unknown_kind`.
const TRIGGER_KINDS: [(&str, &str); 6] = [("explicit", "run.now"), ("scheduled", "schedule.tick"), ("outcome", "run.closed"), ("outcome", "release.published"), ("outcome", "release.promoted"), ("outcome", "release.revoked")];
const SUBJECT_IDS: [&str; 9] = ["run_id", "agent", "release", "outcome", "closed_by", "release_id", "proposal_id", "candidate_hash", "origin"];
const SUBJECT_NUMS: [&str; 2] = ["interval_secs", "slot"];

/// An identifier, never prose: short, no whitespace, no markup.
fn opaque_id(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.is_empty() && s.len() <= 200 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '@' | '/')))
}

fn trigger_key_ok(k: &str) -> bool {
    k.strip_prefix("sha256:").is_some_and(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
}

fn invalid(field: &str) -> Resp {
    problem("validation_error", 422, json!({"field_errors": [{"field": field, "code": "invalid"}]}))
}

impl App {
    /// `pulso.trigger.v1` intake: bearer (route), CSRF, `Idempotency-Key` = `trigger_key`. A new key admits one keyed engine job
    /// (`trigger:<key>`) and writes one audit event; a replay returns the stored response and does neither. Only identifiers
    /// and numbers of the whitelisted subject fields are kept: free text (`reason`) and any other field are dropped and counted.
    fn post_trigger(&self, auto: &Automation, r: &Req) -> Resp {
        if r.headers.get("x-csrf-token") != Some(&self.csrf) {
            return problem("csrf_failed", 403, json!({}));
        }
        let header = r.headers.get("idempotency-key").map(String::as_str).unwrap_or_default();
        if header.is_empty() {
            return invalid("Idempotency-Key");
        }
        let Ok(b) = serde_json::from_slice::<Value>(&r.body) else { return problem("validation_error", 422, json!({})) };
        if b["schema"] != TRIGGER_SCHEMA {
            return invalid("schema");
        }
        let Some(key) = b["trigger_key"].as_str().filter(|k| trigger_key_ok(k)) else { return invalid("trigger_key") };
        if key != header {
            return invalid("Idempotency-Key");
        }
        for f in ["tenant", "mission", "source", "config_digest"] {
            if opaque_id(&b[f]).is_none() {
                return invalid(f);
            }
        }
        let ev = &b["event"];
        if !ev.is_object() || opaque_id(&ev["ref"]).is_none() || !ev.get("subject").is_none_or(Value::is_object) {
            return invalid("event");
        }
        let Some(kind) = b["kind"].as_str() else { return invalid("kind") };
        let ty = ev["type"].as_str().unwrap_or_default();
        if !TRIGGER_KINDS.contains(&(kind, ty)) {
            return problem("unknown_kind", 422, json!({}));
        }
        if b["tenant"] != self.cfg.tenant.as_str() {
            return problem("tenant_mismatch", 403, json!({}));
        }
        let Some(admitter) = auto.admitter.as_ref() else { return problem("admission_unavailable", 503, json!({})) };

        // `requested_at` is the delivery time of this attempt, not part of the trigger: a retry must replay, not conflict.
        let mut canon = b.clone();
        canon.as_object_mut().map(|o| o.remove("requested_at"));
        let digest: String = Sha256::digest(canon.to_string().as_bytes()).iter().map(|x| format!("{x:02x}")).collect();
        let mut trg = auto.triggers.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(rec) = trg.by_key.get(key) {
            if rec.digest != digest {
                return problem("idempotency_conflict", 409, json!({}));
            }
            return resp(202, &rec.response, vec![("Idempotency-Replayed".into(), "true".into())]);
        }

        let (mut subject, mut dropped) = (serde_json::Map::new(), 0u64);
        for (k, v) in ev.get("subject").and_then(Value::as_object).into_iter().flatten() {
            let kept = if SUBJECT_IDS.contains(&k.as_str()) {
                opaque_id(v).map(|s| json!(s))
            } else if SUBJECT_NUMS.contains(&k.as_str()) {
                v.as_u64().map(|n| json!(n))
            } else {
                None
            };
            match kept {
                Some(x) => {
                    subject.insert(k.clone(), x);
                }
                None => dropped += 1,
            }
        }
        let job_key = format!("{TRIGGER_JOB_PREFIX}{key}");
        let Ok(job_id) = admitter.admit_keyed(&self.cfg.tenant, &job_key) else { return problem("admission_failed", 503, json!({"retryable": true})) };

        let ts = |v: &Value| opaque_id(v).map_or(Value::Null, |s| json!(s));
        let view = json!({"trigger_key": key, "kind": kind, "event_type": ty, "ref": ev["ref"], "tenant": b["tenant"], "mission": b["mission"], "source": b["source"],
            "config_digest": b["config_digest"], "subject": Value::Object(subject), "event_at": ts(&ev["at"]), "requested_at": ts(&b["requested_at"]),
            "job_id": job_id, "job_key": job_key, "state": "admitted", "received_at": crate::store::now_iso()});
        let mut audit = view.clone();
        audit["dropped_fields"] = json!(dropped);
        self.audit("automation_trigger_received", key, audit);
        let response = json!({"state": "admitted", "trigger_key": key, "kind": kind, "event_type": ty, "job_id": job_id, "job_key": job_key});
        trg.order.push(key.to_string());
        trg.by_key.insert(key.to_string(), TriggerRec { digest, response: response.clone(), view });
        resp(202, &response, vec![])
    }
}
