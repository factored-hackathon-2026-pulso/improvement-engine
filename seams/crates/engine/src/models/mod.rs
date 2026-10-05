//! `ModelPort`: the one seam through which scout, verifier and builder steps reach a model.
//! Implementations: `Scripted` (fixed answers, no model), `Roleplay` (replay of the roleplay-llm queue), `Gateway`
//! (HTTP to an llm-gateway-compatible endpoint). Every call is recorded with the label and model id of the port that
//! handled it, so a report never calls an answer `real` unless the Gateway actually answered it.
pub mod gateway;
pub mod llm_gateway;
pub mod roleplay;
pub mod scripted;
pub mod tps;

pub use scripted::Scripted;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Scout,
    Verifier,
    Builder,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Scout => "scout",
            Role::Verifier => "verifier",
            Role::Builder => "builder",
        }
    }
}

/// What the payload is made of. `E0` and `Original` never leave the machine toward a hosted model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataClass {
    Synthetic,
    Treated,
    E0,
    Original,
}

impl DataClass {
    pub fn as_str(self) -> &'static str {
        match self {
            DataClass::Synthetic => "synthetic",
            DataClass::Treated => "treated",
            DataClass::E0 => "e0",
            DataClass::Original => "original",
        }
    }
    pub fn may_reach_hosted_model(self) -> bool {
        matches!(self, DataClass::Synthetic | DataClass::Treated)
    }
}

/// Who answered. Only `Gateway` can ever be reported `real`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Scripted,
    Roleplay,
    LocalModel,
    Gateway,
}

impl Label {
    pub fn as_str(self) -> &'static str {
        match self {
            Label::Scripted => "scripted",
            Label::Roleplay => "roleplay",
            Label::LocalModel => "local-model",
            Label::Gateway => "gateway",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub role: Role,
    pub system: String,
    /// The treated agent input dict (TPS shape).
    pub payload: Value,
    /// Exact tokens the stage registered for the TPS scan (signal ids, labels, metric ids).
    pub registry: Vec<String>,
    pub data_class: DataClass,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelAnswer {
    pub content: Value,
    pub model_id: String,
    pub label: Label,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// Policy refused before anything was sent (data class, TPS, not configured).
    Refused(String),
    /// The endpoint or queue could not answer (down, timeout, replay miss).
    Unavailable(String),
    /// The answer is not usable (malformed, forbidden fields).
    Invalid(String),
}

pub trait ModelPort {
    fn label(&self) -> Label;
    fn model_id(&self) -> String;
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Answered,
    Refused(String),
    Unavailable(String),
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    pub role: Role,
    pub label: Label,
    pub model_id: String,
    pub data_class: DataClass,
    pub outcome: Outcome,
}

impl CallRecord {
    pub fn answered(&self) -> bool {
        self.outcome == Outcome::Answered
    }

    /// (report step status, receipt provider). `real` only for an answered Gateway call; a refusal or an outage is
    /// `blocked(model_<why>)`, never a silent fallback to another port.
    pub fn status_provider(&self) -> (String, String) {
        match (&self.outcome, self.label) {
            (Outcome::Answered, Label::Gateway) => ("real".into(), format!("gateway:{}", self.model_id)),
            (Outcome::Answered, Label::Scripted) => ("stand-in".into(), "scripted".into()),
            (Outcome::Answered, Label::Roleplay) => ("stand-in".into(), "agent-roleplay".into()),
            (Outcome::Answered, Label::LocalModel) => ("stand-in".into(), "local-model".into()),
            (Outcome::Refused(_), l) => ("blocked(model_refused)".into(), l.as_str().into()),
            (Outcome::Unavailable(_), l) => ("blocked(model_unavailable)".into(), l.as_str().into()),
            (Outcome::Invalid(_), l) => ("blocked(model_invalid)".into(), l.as_str().into()),
        }
    }

    pub fn to_json(&self) -> Value {
        let (status, provider) = self.status_provider();
        let (outcome, why) = match &self.outcome {
            Outcome::Answered => ("answered", String::new()),
            Outcome::Refused(w) => ("refused", w.clone()),
            Outcome::Unavailable(w) => ("unavailable", w.clone()),
            Outcome::Invalid(w) => ("invalid", w.clone()),
        };
        json!({"role": self.role.as_str(), "label": self.label.as_str(), "model_id": self.model_id, "data_class": self.data_class.as_str(),
               "outcome": outcome, "why": why, "status": status, "provider": provider, "real": status == "real"})
    }
}

/// Wraps a port and records every call (answered or not) under the inner port's label and model id.
pub struct Recording {
    inner: Rc<dyn ModelPort>,
    calls: RefCell<Vec<CallRecord>>,
}

impl Recording {
    pub fn new(inner: Rc<dyn ModelPort>) -> Recording {
        Recording { inner, calls: RefCell::new(vec![]) }
    }

    pub fn calls(&self) -> Vec<CallRecord> {
        self.calls.borrow().clone()
    }

    /// `doubles[]`-shaped entries for every model call that is not `real`.
    pub fn doubles(&self) -> Vec<Value> {
        self.calls()
            .iter()
            .filter(|c| !c.to_json()["real"].as_bool().unwrap_or(false))
            .map(|c| {
                let j = c.to_json();
                json!({"part": format!("model.{}", c.role.as_str()), "status": j["status"], "provider": j["provider"], "model_id": c.model_id, "label": c.label.as_str()})
            })
            .collect()
    }
}

impl ModelPort for Recording {
    fn label(&self) -> Label {
        self.inner.label()
    }
    fn model_id(&self) -> String {
        self.inner.model_id()
    }
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        let r = self.inner.call(req);
        let (label, model_id, outcome) = match &r {
            Ok(a) => (a.label, a.model_id.clone(), Outcome::Answered),
            Err(ModelError::Refused(w)) => (self.inner.label(), self.inner.model_id(), Outcome::Refused(w.clone())),
            Err(ModelError::Unavailable(w)) => (self.inner.label(), self.inner.model_id(), Outcome::Unavailable(w.clone())),
            Err(ModelError::Invalid(w)) => (self.inner.label(), self.inner.model_id(), Outcome::Invalid(w.clone())),
        };
        self.calls.borrow_mut().push(CallRecord { role: req.role, label, model_id, data_class: req.data_class, outcome });
        r
    }
}

/// Cap on the serialized treated payload (and the system prompt) of one call.
pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;

/// Policy shared by every port that talks to a third party (hosted or not): E0/original data is never sent, and the
/// payload must pass the treated-payload scan. Both refusals happen before anything is read or sent.
pub(crate) fn guard(req: &ModelRequest) -> Result<(), ModelError> {
    if !req.data_class.may_reach_hosted_model() {
        return Err(ModelError::Refused(format!("data_class {} never reaches a hosted model", req.data_class.as_str())));
    }
    if req.payload.to_string().len() > MAX_PAYLOAD_BYTES || req.system.len() > MAX_PAYLOAD_BYTES {
        return Err(ModelError::Refused(format!("payload too large (cap {MAX_PAYLOAD_BYTES} bytes)")));
    }
    let scan = tps::scan_payload(&req.payload, tps::DEFAULT_K, &req.registry);
    if !scan.ok {
        return Err(ModelError::Refused(format!("tps: {}", scan.violations.join("; "))));
    }
    Ok(())
}
