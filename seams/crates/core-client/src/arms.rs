//! Evaluation arms DTOs: `ArmRequest` (annex D.4, with the deprecated aliases of ADR 0011) and `ArmReport`.
use crate::canon;
use crate::dto::{DecodeError, err, nullable_str, obj, opt_arr, opt_str, put, req_str};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionProfile {
    AttentionStatefulComplementary,
    EvolutionTask,
}

impl ExecutionProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionProfile::AttentionStatefulComplementary => "attention_stateful_complementary",
            ExecutionProfile::EvolutionTask => "evolution_task",
        }
    }
    fn mode(self) -> ArmMode {
        match self {
            ExecutionProfile::AttentionStatefulComplementary => ArmMode::StatefulAttention,
            ExecutionProfile::EvolutionTask => ArmMode::Native,
        }
    }
}

/// Deprecated `mode` alias (ADR 0011). `task_builder` has no annex profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmMode {
    Native,
    TaskBuilder,
    StatefulAttention,
}

impl ArmMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ArmMode::Native => "native",
            ArmMode::TaskBuilder => "task_builder",
            ArmMode::StatefulAttention => "stateful_attention",
        }
    }
    fn profile(self) -> Option<ExecutionProfile> {
        match self {
            ArmMode::Native => Some(ExecutionProfile::EvolutionTask),
            ArmMode::StatefulAttention => Some(ExecutionProfile::AttentionStatefulComplementary),
            ArmMode::TaskBuilder => None,
        }
    }
}

/// Annex D.4 `ArmRequest`. Either `execution_profile` (annex) or the deprecated `mode` must be set; when both are
/// set they must agree. The key is sent in the `Idempotency-Key` header and copied into the body (spec-allowed).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmRequest {
    pub idempotency_key: String,
    pub binding_ref: String,
    pub case_ref: String,
    pub campaign_ref: Option<String>,
    pub arm: String,
    pub repetition: u32,
    /// Integer or string.
    pub seed: Value,
    /// A JSON object (`{"kind": "published_release" | "frozen_candidate", ...}`).
    pub target: Value,
    pub target_commitment: Option<String>,
    pub scenario_manifest_ref: String,
    pub budget_ref: String,
    pub execution_profile: Option<ExecutionProfile>,
    pub sandbox_session_ref: Option<String>,
    pub oracle_ref: Option<String>,
    /// UTC RFC3339 `Z`; a per-attempt bound, never part of the single-flight identity.
    pub deadline: Option<String>,
    pub supersedes_execution_id: Option<String>,
    // deprecated aliases
    pub mode: Option<ArmMode>,
    pub seed_manifest_ref: Option<String>,
    pub agent_id: Option<String>,
}

fn key_ok(k: &str) -> bool {
    (1..=200).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}

impl ArmRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        idempotency_key: &str,
        binding_ref: &str,
        case_ref: &str,
        arm: &str,
        repetition: u32,
        seed: Value,
        target: Value,
        scenario_manifest_ref: &str,
        budget_ref: &str,
    ) -> ArmRequest {
        ArmRequest {
            idempotency_key: idempotency_key.into(),
            binding_ref: binding_ref.into(),
            case_ref: case_ref.into(),
            campaign_ref: None,
            arm: arm.into(),
            repetition,
            seed,
            target,
            target_commitment: None,
            scenario_manifest_ref: scenario_manifest_ref.into(),
            budget_ref: budget_ref.into(),
            execution_profile: None,
            sandbox_session_ref: None,
            oracle_ref: None,
            deadline: None,
            supersedes_execution_id: None,
            mode: None,
            seed_manifest_ref: None,
            agent_id: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !key_ok(&self.idempotency_key) {
            return Err("idempotency_key must match [A-Za-z0-9_.:-]{1,200}".into());
        }
        for (n, v, max) in [("binding_ref", &self.binding_ref, 200), ("case_ref", &self.case_ref, 200), ("arm", &self.arm, 64)] {
            if v.is_empty() || crate::dto::cp_len(v) > max {
                return Err(format!("{n} must be 1..={max} chars"));
            }
        }
        if self.campaign_ref.as_deref().is_some_and(|c| c.is_empty() || crate::dto::cp_len(c) > 200) {
            return Err("campaign_ref must be 1..=200 chars".into());
        }
        if self.scenario_manifest_ref.is_empty() || self.budget_ref.is_empty() {
            return Err("scenario_manifest_ref and budget_ref are required".into());
        }
        match (&self.execution_profile, &self.mode) {
            (None, None) => return Err("execution_profile is required (or the deprecated mode)".into()),
            (Some(p), Some(m)) if p.mode() != *m => return Err("mode contradicts execution_profile".into()),
            _ => {}
        }
        if let (Some(a), Some(b)) = (&self.sandbox_session_ref, &self.seed_manifest_ref) {
            if a != b {
                return Err("seed_manifest_ref contradicts sandbox_session_ref".into());
            }
        }
        if !self.target.is_object() {
            return Err("target must be a JSON object".into());
        }
        if !(self.seed.is_i64() || self.seed.is_u64() || self.seed.is_string()) {
            return Err("seed must be an integer or a string".into());
        }
        if self.deadline.as_deref().is_some_and(|d| !canon::is_z_timestamp(d)) {
            return Err("deadline must be UTC RFC3339 with a literal Z".into());
        }
        Ok(())
    }

    fn run_mode(&self) -> Option<ArmMode> {
        self.mode.or(self.execution_profile.map(ExecutionProfile::mode))
    }

    fn session_ref(&self) -> Option<&str> {
        self.sandbox_session_ref.as_deref().or(self.seed_manifest_ref.as_deref())
    }

    /// The exact wire body (absent optionals omitted).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        let s = |v: &str| Value::String(v.into());
        m.insert("schema_version".into(), s("1"));
        m.insert("idempotency_key".into(), s(&self.idempotency_key));
        m.insert("binding_ref".into(), s(&self.binding_ref));
        m.insert("case_ref".into(), s(&self.case_ref));
        put(&mut m, "campaign_ref", self.campaign_ref.as_deref());
        m.insert("arm".into(), s(&self.arm));
        m.insert("repetition".into(), Value::from(self.repetition));
        m.insert("seed".into(), self.seed.clone());
        m.insert("target".into(), self.target.clone());
        put(&mut m, "target_commitment", self.target_commitment.as_deref());
        m.insert("scenario_manifest_ref".into(), s(&self.scenario_manifest_ref));
        m.insert("budget_ref".into(), s(&self.budget_ref));
        put(&mut m, "execution_profile", self.execution_profile.map(ExecutionProfile::as_str));
        put(&mut m, "sandbox_session_ref", self.sandbox_session_ref.as_deref());
        put(&mut m, "oracle_ref", self.oracle_ref.as_deref());
        put(&mut m, "deadline", self.deadline.as_deref());
        put(&mut m, "supersedes_execution_id", self.supersedes_execution_id.as_deref());
        put(&mut m, "mode", self.mode.map(ArmMode::as_str));
        put(&mut m, "seed_manifest_ref", self.seed_manifest_ref.as_deref());
        put(&mut m, "agent_id", self.agent_id.as_deref());
        Value::Object(m)
    }

    /// What the bridge's single-flight digest covers (`ArmRequest.canonical()` of `arms.py`): names normalised so the
    /// alias and annex spellings are one request; `deadline` excluded (a per-attempt bound); `agent_id` always filled
    /// (given value, else the one derived from the target) so omitting it equals sending it.
    pub fn single_flight_canonical(&self, derived_agent_id: Option<&str>) -> Value {
        let opt = |v: &Option<String>| v.as_deref().map_or(Value::Null, |s| Value::String(s.into()));
        let mode = self.run_mode();
        let profile = mode.and_then(ArmMode::profile);
        let mut m = Map::new();
        m.insert("schema_version".into(), Value::String("1".into()));
        m.insert("idempotency_key".into(), Value::String(self.idempotency_key.clone()));
        m.insert("binding_ref".into(), Value::String(self.binding_ref.clone()));
        m.insert("case_ref".into(), Value::String(self.case_ref.clone()));
        m.insert("campaign_ref".into(), opt(&self.campaign_ref));
        m.insert("arm".into(), Value::String(self.arm.clone()));
        m.insert("repetition".into(), Value::from(self.repetition));
        m.insert("seed".into(), self.seed.clone());
        m.insert("target".into(), self.target.clone());
        m.insert("target_commitment".into(), opt(&self.target_commitment));
        m.insert("scenario_manifest_ref".into(), Value::String(self.scenario_manifest_ref.clone()));
        m.insert("oracle_ref".into(), opt(&self.oracle_ref));
        m.insert("budget_ref".into(), Value::String(self.budget_ref.clone()));
        m.insert("supersedes_execution_id".into(), opt(&self.supersedes_execution_id));
        m.insert(
            "agent_id".into(),
            self.agent_id.as_deref().or(derived_agent_id).map_or(Value::Null, |s| Value::String(s.into())),
        );
        m.insert("execution_profile".into(), profile.map_or(Value::Null, |p| Value::String(p.as_str().into())));
        if profile.is_none() {
            if let Some(md) = mode {
                m.insert("mode".into(), Value::String(md.as_str().into()));
            }
        }
        m.insert("sandbox_session_ref".into(), self.session_ref().map_or(Value::Null, |s| Value::String(s.into())));
        Value::Object(m)
    }

    /// Local mirror of the bridge's single-flight digest (used to reason about replays client-side; the bridge
    /// remains the authority).
    pub fn single_flight_digest(&self, derived_agent_id: Option<&str>) -> Result<String, canon::CanonError> {
        canon::digest_json(&self.single_flight_canonical(derived_agent_id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmStatus {
    Completed,
    CandidateFailed,
    FailedInfra,
    Unknown,
}

impl ArmStatus {
    fn parse(s: &str) -> Option<ArmStatus> {
        Some(match s {
            "completed" => ArmStatus::Completed,
            "candidate_failed" => ArmStatus::CandidateFailed,
            "failed_infra" => ArmStatus::FailedInfra,
            "unknown" => ArmStatus::Unknown,
            _ => return None,
        })
    }
}

/// `ArmReport`. A denied or interrupted arm reads back as the minimal `{execution_id, status, reason}`, so every
/// other field is optional. `usage == None` is "not available" and `cost_known == Some(false)` is not free.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmReport {
    pub execution_id: String,
    pub status: ArmStatus,
    pub reason: Option<String>,
    pub arm: Option<String>,
    pub case_ref: Option<String>,
    pub repetition: Option<i64>,
    pub seed: Option<Value>,
    pub closed_early: Option<bool>,
    pub closed_early_runs: Vec<String>,
    pub cost_known: Option<bool>,
    pub usage: Option<Value>,
    pub target_commitment: Option<String>,
    pub trace_id: Option<String>,
    pub event_refs: Vec<String>,
    pub effect_receipts: Vec<Value>,
    pub final_state_ref: Option<String>,
    pub initial_state_digest: Option<String>,
    pub oracle_ref: Option<String>,
    pub raw: Value,
}

impl ArmReport {
    pub fn from_json(body: &Value) -> Result<ArmReport, DecodeError> {
        const W: &str = "ArmReport";
        let m = obj(W, body)?;
        let status_s = req_str(W, m, "status")?;
        let status = ArmStatus::parse(&status_s).ok_or_else(|| DecodeError(format!("{W}: unknown status {status_s:?}")))?;
        let strs = |k: &str| opt_arr(m, k).iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>();
        let opt_null = |k: &str| -> Result<Option<String>, DecodeError> {
            if m.contains_key(k) { nullable_str(W, m, k) } else { Ok(None) }
        };
        let usage = match m.get("usage") {
            None | Some(Value::Null) => None,
            Some(u) if u.is_object() => Some(u.clone()),
            Some(_) => return err(W, "`usage` is not an object or null"),
        };
        Ok(ArmReport {
            execution_id: req_str(W, m, "execution_id")?,
            status,
            reason: opt_null("reason")?,
            arm: opt_str(m, "arm"),
            case_ref: opt_str(m, "case_ref"),
            repetition: m.get("repetition").and_then(Value::as_i64),
            seed: m.get("seed").cloned(),
            closed_early: m.get("closed_early").and_then(Value::as_bool),
            closed_early_runs: strs("closed_early_runs"),
            cost_known: m.get("cost_known").and_then(Value::as_bool),
            usage,
            target_commitment: opt_null("target_commitment")?,
            trace_id: opt_str(m, "trace_id"),
            event_refs: strs("event_refs"),
            effect_receipts: opt_arr(m, "effect_receipts"),
            final_state_ref: opt_null("final_state_ref")?,
            initial_state_digest: opt_null("initial_state_digest")?,
            oracle_ref: opt_null("oracle_ref")?,
            raw: body.clone(),
        })
    }

    pub fn is_completed(&self) -> bool {
        self.status == ArmStatus::Completed
    }

    /// `^arm-[0-9a-f]{32}$`, or, when the client opted in for golden tests, a placeholder (`<...>`).
    pub(crate) fn execution_id_well_formed(&self, allow_placeholder: bool) -> bool {
        let id = &self.execution_id;
        (allow_placeholder && id.starts_with('<') && id.ends_with('>'))
            || (id.len() == 36 && id.starts_with("arm-") && id[4..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    }
}
