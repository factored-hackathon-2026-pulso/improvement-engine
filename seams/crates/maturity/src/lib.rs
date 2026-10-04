//! Case-type maturity (pure). RED skeleton.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    E0Treated,
    SimDraftStream,
    PlatformEvents,
}
impl Source {
    pub fn as_str(&self) -> &'static str {
        unimplemented!()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    S0,
    S1,
    S2,
    S3,
    Agent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Thresholds {
    pub repeat_q_min_cases: u32,
    pub tool_use_min: f64,
    pub draft_accept_min: f64,
    pub draft_window: u32,
    pub k_min: u32,
}
impl Default for Thresholds {
    fn default() -> Self {
        unimplemented!()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    AsIs,
    Minor,
    Discarded,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TypeInputs {
    pub copilot_questions: Option<u32>,
    pub repeat_q_cases: Option<u32>,
    pub tool_applicable: Option<u32>,
    pub tool_used: Option<u32>,
    pub drafts: Option<Vec<Disposition>>,
    pub has_agent: bool,
    pub agent_handled: Option<u32>,
    pub agent_resolved: Option<u32>,
    pub cases_today: Option<u32>,
}

pub trait InputSource {
    fn source(&self) -> Source;
    fn simulated(&self) -> bool;
    fn type_ids(&self) -> Vec<String>;
    fn inputs(&self, type_id: &str) -> Option<TypeInputs>;
}

pub struct JsonSource;
impl JsonSource {
    pub fn from_json(_source: Source, _simulated: bool, _v: &Value) -> Result<JsonSource, String> {
        unimplemented!()
    }
}
impl InputSource for JsonSource {
    fn source(&self) -> Source {
        unimplemented!()
    }
    fn simulated(&self) -> bool {
        unimplemented!()
    }
    fn type_ids(&self) -> Vec<String> {
        unimplemented!()
    }
    fn inputs(&self, _t: &str) -> Option<TypeInputs> {
        unimplemented!()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Metric {
    Value { value: f64, numerator: u32, denominator: u32, source: Source, simulated: bool },
    NotComputable { reason: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Maturity {
    pub type_id: String,
    pub stage: Stage,
    pub agent_proposed: bool,
    pub simulated: bool,
    pub repeat_q: Metric,
    pub tool_use_rate: Metric,
    pub draft_accept_100: Metric,
    pub agent_resolved: Metric,
    pub blocked_by: Option<String>,
    pub cases_today: Option<u32>,
}
impl Maturity {
    pub fn to_json(&self, _t: &Thresholds) -> Value {
        unimplemented!()
    }
}

/// Sources are tried in order per metric: the first that can compute wins.
pub fn evaluate(_type_id: &str, _t: &Thresholds, _sources: &[&dyn InputSource]) -> Maturity {
    unimplemented!()
}
pub fn thresholds_to_json(_t: &Thresholds) -> Value {
    unimplemented!()
}
pub fn thresholds_from_json(_base: &Thresholds, _v: &Value) -> Result<Thresholds, String> {
    unimplemented!()
}
