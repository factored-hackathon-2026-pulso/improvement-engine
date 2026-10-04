//! /internal/v1/automation: the read model behind the platform's "Automatizacion" screens (case-type maturity) plus the
//! team thresholds and a SIMULATED approve / publish-to-staging path. Keyed on the agent-core proposal id and its target
//! artifact (`agent_id@alias`); nothing here is console specific. Metrics come from the pure `maturity` crate; every
//! number carries its source and a `simulated` flag, and what cannot be computed says so.
use crate::app::{App, Req, Resp, bearer_ok, ok, problem, resp};
use crate::event::{NewEvent, RunEventSink};
use maturity::{Disposition, InputSource, JsonSource, Maturity, Metric, Stage, Thresholds};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;

pub const AUTOMATION_PREFIX: &str = "/internal/v1/automation";
const AUDIT_RUN: &str = "automation-audit";
const STAGE_KEYS: [&str; 4] = ["1", "2", "3", "agent"];

pub struct Automation {
    e0: Option<JsonSource>,
    sim: Option<JsonSource>,
    as_of: String,
    proposals: Value,
    state: Mutex<State>,
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
        })
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
        json!({"as_of": self.as_of, "data_origin": self.origin(), "doubles": self.doubles(), "thresholds": maturity::thresholds_to_json(&st.thresholds), "revision": st.revision, "banner": banner, "assumptions": self.assumptions(), "case_types": items})
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
