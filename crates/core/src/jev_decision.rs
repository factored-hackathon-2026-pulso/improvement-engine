//! Typed M5-compatible decision consumer boundary; no Jev/provider runtime.

use crate::core_task::{CoreTaskBinding, CoreTaskScope};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionModelRef {
    id: String,
    version: String,
    digest: String,
}
impl DecisionModelRef {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        digest: impl Into<String>,
    ) -> Result<Self, JevDecisionError> {
        let id = id.into();
        let version = version.into();
        let digest = digest.into();
        validate_identifier(&id, "decision_model_id")?;
        validate_semver(&version)?;
        if !is_sha256(&digest) {
            return Err(JevDecisionError::InvalidDecisionModelRef);
        }
        Ok(Self {
            id,
            version,
            digest,
        })
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// A sealed M5 calibration/threshold row for one `(locale, choice)`.
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationRule {
    locale: String,
    choice: String,
    p_raw: f64,
    p_cal: f64,
    threshold: f64,
}
impl CalibrationRule {
    pub fn new(
        locale: impl Into<String>,
        choice: impl Into<String>,
        p_raw: f64,
        p_cal: f64,
        threshold: f64,
    ) -> Result<Self, JevDecisionError> {
        let locale = locale.into();
        let choice = choice.into();
        validate_identifier(&locale, "locale")?;
        validate_identifier(&choice, "choice")?;
        if [p_raw, p_cal, threshold]
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(JevDecisionError::InvalidCalibration);
        }
        Ok(Self {
            locale,
            choice,
            p_raw,
            p_cal,
            threshold,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionPolicy {
    model: DecisionModelRef,
    jev_version: String,
    model_version: String,
    calibration_ref: String,
    choices: Vec<String>,
    rules: BTreeMap<(String, String), CalibrationRule>,
    digest: String,
}
impl DecisionPolicy {
    pub fn new(
        model: DecisionModelRef,
        jev_version: impl Into<String>,
        model_version: impl Into<String>,
        calibration_ref: impl Into<String>,
        choices: Vec<impl Into<String>>,
        rules: Vec<CalibrationRule>,
    ) -> Result<Self, JevDecisionError> {
        let jev_version = jev_version.into();
        let model_version = model_version.into();
        let calibration_ref = calibration_ref.into();
        validate_opaque_version(&jev_version, "jev_version")?;
        validate_opaque_version(&model_version, "model_version")?;
        validate_identifier(&calibration_ref, "calibration_ref")?;
        let choices = choices.into_iter().map(Into::into).collect::<Vec<String>>();
        if choices.is_empty()
            || choices
                .iter()
                .any(|v| validate_identifier(v, "choice").is_err())
            || choices.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(JevDecisionError::InvalidPolicy);
        }
        let mut map = BTreeMap::new();
        for rule in rules {
            if !choices.contains(&rule.choice)
                || map
                    .insert((rule.locale.clone(), rule.choice.clone()), rule)
                    .is_some()
            {
                return Err(JevDecisionError::InvalidCalibration);
            }
        }
        let mut canonical = format!(
            "model:{}@{}\nmodel_digest:{}\njev:{}\nmodel_version:{}\ncalibration:{}\n",
            model.id, model.version, model.digest, jev_version, model_version, calibration_ref
        );
        for choice in &choices {
            canonical.push_str(&format!("choice:{choice}\n"));
        }
        for ((locale, choice), rule) in &map {
            canonical.push_str(&format!(
                "rule:{locale}:{choice}:{}:{}:{}\n",
                rule.p_raw.to_bits(),
                rule.p_cal.to_bits(),
                rule.threshold.to_bits()
            ));
        }
        Ok(Self {
            model,
            jev_version,
            model_version,
            calibration_ref,
            choices,
            rules: map,
            digest: format!(
                "jev-policy:sha256:{:x}",
                Sha256::digest(canonical.as_bytes())
            ),
        })
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn model(&self) -> &DecisionModelRef {
        &self.model
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JevDecisionInvocation {
    scope: CoreTaskScope,
    task_binding: CoreTaskBinding,
    policy: DecisionPolicy,
    attempt_id: String,
    locale: String,
    model_view_digest: String,
}
impl JevDecisionInvocation {
    pub fn new(
        scope: CoreTaskScope,
        task_binding: CoreTaskBinding,
        policy: DecisionPolicy,
        attempt_id: impl Into<String>,
        locale: impl Into<String>,
        model_view_digest: impl Into<String>,
    ) -> Result<Self, JevDecisionError> {
        let attempt_id = attempt_id.into();
        let locale = locale.into();
        let model_view_digest = model_view_digest.into();
        validate_identifier(&attempt_id, "attempt_id")?;
        validate_identifier(&locale, "locale")?;
        if !is_sha256(&model_view_digest) {
            return Err(JevDecisionError::InvalidModelViewDigest);
        }
        Ok(Self {
            scope,
            task_binding,
            policy,
            attempt_id,
            locale,
            model_view_digest,
        })
    }
}

/// Provider output carries raw probability. M5 confidence is derived only by
/// the pinned calibration/threshold provenance, never from a provider hint.
#[derive(Debug, Clone, PartialEq)]
pub struct JevStructuredDecision {
    choice: String,
    p_raw: f64,
}
impl JevStructuredDecision {
    pub fn new(choice: impl Into<String>, p_raw: f64) -> Result<Self, JevDecisionError> {
        let choice = choice.into();
        validate_identifier(&choice, "choice")?;
        if !p_raw.is_finite() || !(0.0..=1.0).contains(&p_raw) {
            return Err(JevDecisionError::InvalidStructuredOutput);
        }
        Ok(Self { choice, p_raw })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionRoute {
    Selected(String),
    LowConfidence,
}
#[derive(Debug, Clone, PartialEq)]
pub struct JevDecisionReceipt {
    task_binding_digest: String,
    decision_model_digest: String,
    jev_version: String,
    model_version: String,
    policy_digest: String,
    model_view_digest: String,
    output_digest: String,
    p_raw: f64,
    p_cal: Option<f64>,
    threshold: Option<f64>,
    route: DecisionRoute,
}
impl JevDecisionReceipt {
    pub fn task_binding_digest(&self) -> &str {
        &self.task_binding_digest
    }
    pub fn decision_model_digest(&self) -> &str {
        &self.decision_model_digest
    }
    pub fn jev_version(&self) -> &str {
        &self.jev_version
    }
    pub fn model_version(&self) -> &str {
        &self.model_version
    }
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
    pub fn model_view_digest(&self) -> &str {
        &self.model_view_digest
    }
    pub fn output_digest(&self) -> &str {
        &self.output_digest
    }
    pub fn p_raw(&self) -> f64 {
        self.p_raw
    }
    pub fn p_cal(&self) -> Option<f64> {
        self.p_cal
    }
    pub fn threshold(&self) -> Option<f64> {
        self.threshold
    }
    pub fn route(&self) -> &DecisionRoute {
        &self.route
    }
}
pub trait JevDecisionPort {
    fn decide(
        &mut self,
        invocation: JevDecisionInvocation,
    ) -> Result<JevDecisionReceipt, JevDecisionError>;
}
#[derive(Debug, Clone, PartialEq)]
struct Attempt {
    scope: CoreTaskScope,
    binding: CoreTaskBinding,
    policy: DecisionPolicy,
    locale: String,
    view: String,
    receipt: JevDecisionReceipt,
}
pub struct JevDecisionSimulator {
    binding: CoreTaskBinding,
    policy: DecisionPolicy,
    script: Option<JevStructuredDecision>,
    attempts: BTreeMap<(String, String, String), Attempt>,
    count: u64,
}
impl JevDecisionSimulator {
    pub fn new(binding: CoreTaskBinding, policy: DecisionPolicy) -> Self {
        Self {
            binding,
            policy,
            script: None,
            attempts: BTreeMap::new(),
            count: 0,
        }
    }
    pub fn script(&mut self, value: JevStructuredDecision) {
        self.script = Some(value)
    }
    pub fn decision_count(&self) -> u64 {
        self.count
    }
}
impl JevDecisionPort for JevDecisionSimulator {
    fn decide(
        &mut self,
        invocation: JevDecisionInvocation,
    ) -> Result<JevDecisionReceipt, JevDecisionError> {
        if invocation.task_binding != self.binding {
            return Err(JevDecisionError::UnsupportedTaskBinding);
        }
        if invocation.policy != self.policy {
            return Err(JevDecisionError::UnsupportedPolicy {
                policy_digest: invocation.policy.digest.clone(),
            });
        }
        let key = (
            invocation.scope.tenant_id().to_owned(),
            invocation.scope.job_id().to_owned(),
            invocation.attempt_id.clone(),
        );
        if let Some(old) = self.attempts.get(&key) {
            if old.scope != invocation.scope {
                return Err(JevDecisionError::ScopeMismatch {
                    attempt_id: invocation.attempt_id,
                });
            }
            if old.binding != invocation.task_binding
                || old.policy != invocation.policy
                || old.locale != invocation.locale
                || old.view != invocation.model_view_digest
            {
                return Err(JevDecisionError::AttemptConflict {
                    attempt_id: invocation.attempt_id,
                });
            }
            return Ok(old.receipt.clone());
        }
        let output = self.script.clone().ok_or(JevDecisionError::MissingScript)?;
        if !invocation.policy.choices.contains(&output.choice) {
            return Err(JevDecisionError::InvalidEnumOutput {
                choice: output.choice,
            });
        }
        self.count = self
            .count
            .checked_add(1)
            .ok_or(JevDecisionError::DecisionCountExhausted)?;
        let rule = invocation
            .policy
            .rules
            .get(&(invocation.locale.clone(), output.choice.clone()));
        let (p_cal, threshold, route) = match rule {
            Some(rule)
                if rule.p_raw.to_bits() == output.p_raw.to_bits()
                    && rule.p_cal >= rule.threshold =>
            {
                (
                    Some(rule.p_cal),
                    Some(rule.threshold),
                    DecisionRoute::Selected(output.choice.clone()),
                )
            }
            Some(rule) if rule.p_raw.to_bits() == output.p_raw.to_bits() => (
                Some(rule.p_cal),
                Some(rule.threshold),
                DecisionRoute::LowConfidence,
            ),
            _ => (None, None, DecisionRoute::LowConfidence),
        };
        let receipt = JevDecisionReceipt {
            task_binding_digest: invocation.task_binding.digest().to_owned(),
            decision_model_digest: invocation.policy.model.digest().to_owned(),
            jev_version: invocation.policy.jev_version.clone(),
            model_version: invocation.policy.model_version.clone(),
            policy_digest: invocation.policy.digest.clone(),
            model_view_digest: invocation.model_view_digest.clone(),
            output_digest: output_digest(&output),
            p_raw: output.p_raw,
            p_cal,
            threshold,
            route,
        };
        self.attempts.insert(
            key,
            Attempt {
                scope: invocation.scope,
                binding: invocation.task_binding,
                policy: invocation.policy,
                locale: invocation.locale,
                view: invocation.model_view_digest,
                receipt: receipt.clone(),
            },
        );
        Ok(receipt)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JevDecisionError {
    InvalidIdentifier { field: &'static str },
    InvalidDecisionModelRef,
    InvalidPolicy,
    InvalidCalibration,
    InvalidModelViewDigest,
    InvalidStructuredOutput,
    UnsupportedTaskBinding,
    UnsupportedPolicy { policy_digest: String },
    MissingScript,
    InvalidEnumOutput { choice: String },
    ScopeMismatch { attempt_id: String },
    AttemptConflict { attempt_id: String },
    DecisionCountExhausted,
}
impl fmt::Display for JevDecisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field } => {
                write!(f, "{field} must be a lowercase identifier")
            }
            Self::InvalidDecisionModelRef => {
                f.write_str("decision model ref must include a sha256 digest")
            }
            Self::InvalidPolicy => f.write_str("decision policy enum must be sorted and unique"),
            Self::InvalidCalibration => f.write_str("calibration provenance is invalid"),
            Self::InvalidModelViewDigest => f.write_str("model-view digest must be sha256"),
            Self::InvalidStructuredOutput => {
                f.write_str("provider output probability must be finite from zero to one")
            }
            Self::UnsupportedTaskBinding => f.write_str("task binding unsupported"),
            Self::UnsupportedPolicy { .. } => f.write_str("decision policy unsupported"),
            Self::MissingScript => f.write_str("missing scripted output"),
            Self::InvalidEnumOutput { .. } => f.write_str("output enum is undeclared"),
            Self::ScopeMismatch { .. } => f.write_str("attempt scope mismatch"),
            Self::AttemptConflict { .. } => f.write_str("attempt payload conflict"),
            Self::DecisionCountExhausted => f.write_str("decision count exhausted"),
        }
    }
}
impl std::error::Error for JevDecisionError {}
fn validate_identifier(v: &str, field: &'static str) -> Result<(), JevDecisionError> {
    let mut c = v.chars();
    if !matches!(c.next(),Some(x)if x.is_ascii_lowercase())
        || !c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '_' || x == '-')
    {
        return Err(JevDecisionError::InvalidIdentifier { field });
    }
    Ok(())
}
fn validate_semver(v: &str) -> Result<(), JevDecisionError> {
    let parts = v.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(JevDecisionError::InvalidDecisionModelRef);
    }
    Ok(())
}
fn validate_opaque_version(v: &str, field: &'static str) -> Result<(), JevDecisionError> {
    if v.is_empty()
        || v.len() > 255
        || !v.bytes().all(|byte| {
            matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b':' | b'_' | b'-')
        })
    {
        return Err(JevDecisionError::InvalidIdentifier { field });
    }
    Ok(())
}
fn is_sha256(v: &str) -> bool {
    v.len() == 71
        && v.starts_with("sha256:")
        && v.as_bytes()[7..]
            .iter()
            .all(|b| matches!(*b,b'0'..=b'9'|b'a'..=b'f'))
}
fn output_digest(o: &JevStructuredDecision) -> String {
    format!(
        "jev-output:sha256:{:x}",
        Sha256::digest(
            format!("choice:{}\np_raw_bits:{}\n", o.choice, o.p_raw.to_bits()).as_bytes()
        )
    )
}
