//! Case-type maturity model (pure, no I/O). Per case type: copilot answers (stage 1), proposes tools when the same
//! question repeats over enough cases (stage 2), proposes drafts when the proposed tools are used (stage 3), and an
//! agent is proposed when enough of the last N drafts are sent as is or with minor edits. "Con agente" = an agent runs.
//! Every metric carries its `source` and a `simulated` flag, and is `NotComputable(reason)` when its inputs are missing:
//! nothing is ever fabricated. Inputs are aggregates only (no ids, no free text).
use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    E0Treated,
    SimDraftStream,
    PlatformEvents,
}
impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::E0Treated => "e0_treated",
            Source::SimDraftStream => "sim_draft_stream",
            Source::PlatformEvents => "platform_events",
        }
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
impl Stage {
    fn to_json(self) -> Value {
        match self {
            Stage::S0 => json!(0),
            Stage::S1 => json!(1),
            Stage::S2 => json!(2),
            Stage::S3 => json!(3),
            Stage::Agent => json!("agent"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Thresholds {
    /// Stage 1 -> 2: the same copilot question repeats over at least this many cases.
    pub repeat_q_min_cases: u32,
    /// Stage 2 -> 3: proposed tools used in at least this share of applicable cases.
    pub tool_use_min: f64,
    /// Stage 3 -> agent: share of the last `draft_window` drafts sent as is or with minor edits.
    pub draft_accept_min: f64,
    pub draft_window: u32,
    /// k-anonymity floor: a ratio over fewer than this many cases is suppressed.
    pub k_min: u32,
}
impl Default for Thresholds {
    fn default() -> Self {
        Thresholds { repeat_q_min_cases: 20, tool_use_min: 0.7, draft_accept_min: 0.8, draft_window: 100, k_min: 10 }
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

const ALLOWED: [&str; 15] = [
    "type_id", "label", "group", "stage_since", "copilot_questions", "repeat_q_cases", "tool_applicable", "tool_used", "drafts", "has_agent", "agent_handled", "agent_resolved", "agent_handed", "copilot_cases", "cases_total",
];

/// Aggregates parsed from JSON `{"case_types":[{type_id, ...counts}]}`. Unknown keys (ids, free text) are refused.
pub struct JsonSource {
    source: Source,
    simulated: bool,
    rows: Vec<(String, TypeInputs, Value)>,
}

fn opt_u32(o: &Map<String, Value>, k: &str) -> Result<Option<u32>, String> {
    match o.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).map(Some).ok_or_else(|| format!("{k}: expected a non-negative integer")),
    }
}

impl JsonSource {
    pub fn from_json(source: Source, simulated: bool, v: &Value) -> Result<JsonSource, String> {
        let items = v["case_types"].as_array().ok_or("case_types: expected an array")?;
        let mut rows = Vec::new();
        for it in items {
            let o = it.as_object().ok_or("case_types[]: expected an object")?;
            if let Some(k) = o.keys().find(|k| !ALLOWED.contains(&k.as_str()) && k.as_str() != "cases_today") {
                return Err(format!("{k}: not an allowed aggregate field"));
            }
            let id = o.get("type_id").and_then(Value::as_str).ok_or("type_id: required")?.to_string();
            let drafts = match o.get("drafts") {
                None | Some(Value::Null) => None,
                Some(Value::Array(a)) => Some(
                    a.iter()
                        .map(|d| match d.as_str() {
                            Some("as_is") => Ok(Disposition::AsIs),
                            Some("minor") => Ok(Disposition::Minor),
                            Some("discarded") => Ok(Disposition::Discarded),
                            _ => Err("drafts: values are as_is | minor | discarded".to_string()),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                Some(_) => return Err("drafts: expected an array".into()),
            };
            let i = TypeInputs {
                copilot_questions: opt_u32(o, "copilot_questions")?,
                repeat_q_cases: opt_u32(o, "repeat_q_cases")?,
                tool_applicable: opt_u32(o, "tool_applicable")?,
                tool_used: opt_u32(o, "tool_used")?,
                drafts,
                has_agent: o.get("has_agent").and_then(Value::as_bool).unwrap_or(false),
                agent_handled: opt_u32(o, "agent_handled")?,
                agent_resolved: opt_u32(o, "agent_resolved")?,
                cases_today: opt_u32(o, "cases_today")?,
            };
            rows.push((id, i, it.clone()));
        }
        Ok(JsonSource { source, simulated, rows })
    }
    /// The raw row (label, group, stage_since) for presentation.
    pub fn row(&self, type_id: &str) -> Option<&Value> {
        self.rows.iter().find(|(k, _, _)| k == type_id).map(|(_, _, v)| v)
    }
}
impl InputSource for JsonSource {
    fn source(&self) -> Source {
        self.source
    }
    fn simulated(&self) -> bool {
        self.simulated
    }
    fn type_ids(&self) -> Vec<String> {
        self.rows.iter().map(|(k, _, _)| k.clone()).collect()
    }
    fn inputs(&self, t: &str) -> Option<TypeInputs> {
        self.rows.iter().find(|(k, _, _)| k == t).map(|(_, i, _)| i.clone())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Metric {
    Value { value: f64, numerator: u32, denominator: u32, source: Source, simulated: bool },
    NotComputable { reason: String },
}
impl Metric {
    fn to_json(&self) -> Value {
        match self {
            Metric::Value { value, numerator, denominator, source, simulated } => {
                json!({"status": "ok", "value": value, "numerator": numerator, "denominator": denominator, "source": source.as_str(), "simulated": simulated})
            }
            Metric::NotComputable { reason } => json!({"status": "not_computable", "reason": reason}),
        }
    }
    fn simulated(&self) -> bool {
        matches!(self, Metric::Value { simulated: true, .. })
    }
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

pub fn thresholds_to_json(t: &Thresholds) -> Value {
    json!({"repeat_q_min_cases": t.repeat_q_min_cases, "tool_use_min": t.tool_use_min, "draft_accept_min": t.draft_accept_min, "draft_window": t.draft_window, "k_min": t.k_min})
}

/// Overlay the keys present in `v` on `base`; every value is validated (privacy floor k_min >= 10).
pub fn thresholds_from_json(base: &Thresholds, v: &Value) -> Result<Thresholds, String> {
    let o = v.as_object().ok_or("expected an object")?;
    let mut t = base.clone();
    let count = |k: &str, min: u32| -> Result<Option<u32>, String> {
        match o.get(k) {
            None => Ok(None),
            Some(x) => x.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| *n >= min).map(Some).ok_or_else(|| format!("{k}: integer >= {min}")),
        }
    };
    let share = |k: &str| -> Result<Option<f64>, String> {
        match o.get(k) {
            None => Ok(None),
            Some(x) => x.as_f64().filter(|f| *f > 0.0 && *f <= 1.0).map(Some).ok_or_else(|| format!("{k}: number in (0, 1]")),
        }
    };
    if let Some(n) = count("repeat_q_min_cases", 1)? {
        t.repeat_q_min_cases = n;
    }
    if let Some(n) = count("draft_window", 1)? {
        t.draft_window = n;
    }
    if let Some(n) = count("k_min", 10)? {
        t.k_min = n;
    }
    if let Some(f) = share("tool_use_min")? {
        t.tool_use_min = f;
    }
    if let Some(f) = share("draft_accept_min")? {
        t.draft_accept_min = f;
    }
    if let Some(k) = o.keys().find(|k| !["repeat_q_min_cases", "draft_window", "k_min", "tool_use_min", "draft_accept_min"].contains(&k.as_str())) {
        return Err(format!("{k}: unknown threshold"));
    }
    Ok(t)
}

impl Maturity {
    pub fn to_json(&self, t: &Thresholds) -> Value {
        json!({
            "type_id": self.type_id, "stage": self.stage.to_json(), "agent_proposed": self.agent_proposed, "simulated": self.simulated,
            "blocked_by": self.blocked_by, "cases_today": self.cases_today,
            "metrics": {"repeat_q": self.repeat_q.to_json(), "tool_use_rate": self.tool_use_rate.to_json(), "draft_accept_100": self.draft_accept_100.to_json(), "agent_resolved": self.agent_resolved.to_json()},
            "thresholds": {
                "stage1_to_2": {"metric": "repeat_q", "min": t.repeat_q_min_cases, "unit": "cases"},
                "stage2_to_3": {"metric": "tool_use_rate", "min": t.tool_use_min, "unit": "share"},
                "stage3_to_agent": {"metric": "draft_accept_100", "min": t.draft_accept_min, "unit": "share", "window": t.draft_window},
            },
        })
    }
}

fn nc(reason: &str) -> Metric {
    Metric::NotComputable { reason: reason.to_string() }
}

/// First source able to compute wins; otherwise the first reason given (a source that knows the type), else `no_input`.
fn first(sources: &[(Source, bool, Option<TypeInputs>)], f: impl Fn(&TypeInputs, Source, bool) -> Metric) -> Metric {
    let mut reason: Option<Metric> = None;
    for (s, sim, i) in sources {
        if let Some(i) = i {
            match f(i, *s, *sim) {
                m @ Metric::Value { .. } => return m,
                m => {
                    reason.get_or_insert(m);
                }
            }
        }
    }
    reason.unwrap_or_else(|| nc("no_input"))
}

fn ratio(n: u32, d: u32, source: Source, simulated: bool) -> Metric {
    Metric::Value { value: if d == 0 { 0.0 } else { f64::from(n) / f64::from(d) }, numerator: n, denominator: d, source, simulated }
}

/// Sources are tried in order per metric: the first that can compute wins.
pub fn evaluate(type_id: &str, t: &Thresholds, sources: &[&dyn InputSource]) -> Maturity {
    let all: Vec<(Source, bool, Option<TypeInputs>)> = sources.iter().map(|s| (s.source(), s.simulated(), s.inputs(type_id))).collect();
    let repeat_q = first(&all, |i, s, sim| match i.repeat_q_cases {
        Some(n) => Metric::Value { value: f64::from(n), numerator: n, denominator: n, source: s, simulated: sim },
        None => nc("no_copilot_query_signature"),
    });
    let tool_use_rate = first(&all, |i, s, sim| match (i.tool_used, i.tool_applicable) {
        (Some(u), Some(a)) if a >= t.k_min => ratio(u, a, s, sim),
        (Some(_), Some(a)) => Metric::NotComputable { reason: format!("below_k_min: {a} < {}", t.k_min) },
        _ => nc("no_tool_use_events"),
    });
    let draft_accept_100 = first(&all, |i, s, sim| match &i.drafts {
        None => nc("no_draft_rows"),
        Some(d) if (d.len() as u32) < t.draft_window => Metric::NotComputable { reason: format!("window_incomplete: {} of {}", d.len(), t.draft_window) },
        Some(d) => {
            let last = &d[d.len() - t.draft_window as usize..];
            ratio(last.iter().filter(|x| **x != Disposition::Discarded).count() as u32, t.draft_window, s, sim)
        }
    });
    let agent_resolved = first(&all, |i, s, sim| match (i.has_agent, i.agent_resolved, i.agent_handled) {
        (true, Some(r), Some(h)) => ratio(r, h, s, sim),
        _ => nc("no_agent_run"),
    });
    let has_agent = all.iter().any(|(_, _, i)| i.as_ref().is_some_and(|i| i.has_agent));
    let copilot = all.iter().find_map(|(_, sim, i)| i.as_ref().and_then(|i| i.copilot_questions.map(|q| (q, *sim))));
    let cases_today = all.iter().find_map(|(_, _, i)| i.as_ref().and_then(|i| i.cases_today));

    let pass = |m: &Metric, min: f64| matches!(m, Metric::Value { value, .. } if *value >= min);
    let blocked = |name: &str, m: &Metric| match m {
        Metric::NotComputable { reason } => Some(format!("{name}: {reason}")),
        _ => None,
    };
    let (mut stage, mut agent_proposed, mut blocked_by) = (Stage::S0, false, None);
    if has_agent {
        stage = Stage::Agent;
    } else if copilot.is_some_and(|(q, _)| q > 0) {
        stage = Stage::S1;
        if pass(&repeat_q, f64::from(t.repeat_q_min_cases)) {
            stage = Stage::S2;
            if pass(&tool_use_rate, t.tool_use_min) {
                stage = Stage::S3;
                agent_proposed = pass(&draft_accept_100, t.draft_accept_min);
                if !agent_proposed {
                    blocked_by = blocked("draft_accept_100", &draft_accept_100);
                }
            } else {
                blocked_by = blocked("tool_use_rate", &tool_use_rate);
            }
        } else {
            blocked_by = blocked("repeat_q", &repeat_q);
        }
    } else if all.iter().any(|(_, _, i)| i.is_some()) && copilot.is_none() {
        blocked_by = Some("copilot_questions: no_input".into());
    }
    let simulated = stage != Stage::S0
        && (copilot.is_some_and(|(_, s)| s) || repeat_q.simulated() || tool_use_rate.simulated() || (stage >= Stage::S3 && draft_accept_100.simulated()) || (stage == Stage::Agent && agent_resolved.simulated()));
    Maturity { type_id: type_id.to_string(), stage, agent_proposed, simulated, repeat_q, tool_use_rate, draft_accept_100, agent_resolved, blocked_by, cases_today }
}
