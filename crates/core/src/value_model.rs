//! Bounded, evidence-labeled scenario valuation and deterministic portfolio ranking.
//!
//! The amounts produced here are scenario estimates, never observed savings or
//! evidence that an intervention caused a bank outcome. Integer cents and basis
//! points avoid floating-point drift and make every rounding decision explicit.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser::SerializeStruct};
use sha2::{Digest, Sha256};

const RATE_SCALE: u128 = 10_000;

/// Provenance categories from the engine's envelope contract.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    SuppliedSynthetic,
    TeamGenerated,
    ExternalAssumption,
    FutureBank,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidencePhase {
    Discovery,
    ReproductionHoldout,
    Other,
}

/// Identifies the run and source snapshot behind a factor. The reference strings
/// are lineage pointers, not proof of authenticity; the E0 source adapter must
/// verify them before constructing trusted local inputs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBinding {
    pub run_digest: String,
    pub source_snapshot_digest: String,
    pub evidence_digest: String,
    pub phase: EvidencePhase,
}

/// An integer probability/ratio in basis points, where 10,000 means 100%.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Rate(u32);

impl Rate {
    #[must_use]
    pub const fn from_basis_points(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn basis_points(self) -> u32 {
        self.0
    }
}

/// Currency amounts use signed integer cents internally and decimal strings on
/// the wire, together with an explicit USD currency tag in `UsdRange`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct UsdCents(i128);

impl UsdCents {
    #[must_use]
    pub const fn new(cents: i128) -> Self {
        Self(cents)
    }

    #[must_use]
    pub const fn cents(self) -> i128 {
        self.0
    }

    fn as_decimal_string(self) -> String {
        let negative = self.0 < 0;
        let absolute = self.0.unsigned_abs();
        let whole = absolute / 100;
        let fraction = absolute % 100;
        format!("{}{}.{fraction:02}", if negative { "-" } else { "" }, whole)
    }
}

impl Serialize for UsdCents {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_decimal_string())
    }
}

impl<'de> Deserialize<'de> for UsdCents {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let (negative, digits) = match value.strip_prefix('-') {
            Some(remaining) => (true, remaining),
            None => (false, value.as_str()),
        };
        let (whole, fraction) = digits.split_once('.').ok_or_else(|| {
            de::Error::custom("USD amount must be a decimal string with two places")
        })?;
        if whole.is_empty()
            || fraction.len() != 2
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
            || (whole.len() > 1 && whole.starts_with('0'))
        {
            return Err(de::Error::custom(
                "USD amount is not in canonical decimal form",
            ));
        }
        let whole: i128 = whole.parse().map_err(de::Error::custom)?;
        let fraction: i128 = fraction.parse().map_err(de::Error::custom)?;
        let cents = whole
            .checked_mul(100)
            .and_then(|value| value.checked_add(fraction))
            .ok_or_else(|| de::Error::custom("USD amount overflows cents"))?;
        Ok(Self(if negative { -cents } else { cents }))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FactorRange<T> {
    pub low: T,
    pub base: T,
    pub high: T,
}

/// A missing measurement remains explicitly unknown; it is never coerced to 0.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum FactorEstimate<T> {
    Known {
        evidence_kind: EvidenceKind,
        evidence: EvidenceBinding,
        range: FactorRange<T>,
    },
    Unknown {
        evidence_kind: EvidenceKind,
        evidence: EvidenceBinding,
        reason: UnknownReason,
    },
}

impl<T> FactorEstimate<T> {
    #[must_use]
    pub fn known(
        range: FactorRange<T>,
        evidence_kind: EvidenceKind,
        evidence: EvidenceBinding,
    ) -> Self {
        Self::Known {
            evidence_kind,
            evidence,
            range,
        }
    }

    #[must_use]
    pub fn unknown(
        evidence_kind: EvidenceKind,
        reason: UnknownReason,
        evidence: EvidenceBinding,
    ) -> Self {
        Self::Unknown {
            evidence_kind,
            evidence,
            reason,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    NotMeasured,
    NoSupportedUsdValue,
    NoSupportedEffectEstimate,
    InsufficientPopulationEvidence,
    OverlapNeedsMarginalEvidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RankingCriterion {
    SafetyGate,
    EvaluabilityGate,
    EligibilityGate,
    NetBaseUsdDescending,
    UncertaintyWidthAscending,
    EffortMinutesAscending,
    ConfidenceDescending,
    ScopeDigestAscending,
    CandidateIdAscending,
}

/// Versioned, serializable policy. Field order and enum serialization are part
/// of the canonical digest contract; do not reorder fields without a schema bump.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValueModelConfig {
    pub schema_version: u16,
    pub config_version: String,
    pub ranking_criteria: Vec<RankingCriterion>,
}

impl Default for ValueModelConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            config_version: "scenario-ranking-0.1".to_owned(),
            ranking_criteria: vec![
                RankingCriterion::SafetyGate,
                RankingCriterion::EvaluabilityGate,
                RankingCriterion::EligibilityGate,
                RankingCriterion::NetBaseUsdDescending,
                RankingCriterion::UncertaintyWidthAscending,
                RankingCriterion::EffortMinutesAscending,
                RankingCriterion::ConfidenceDescending,
                RankingCriterion::ScopeDigestAscending,
                RankingCriterion::CandidateIdAscending,
            ],
        }
    }
}

impl ValueModelConfig {
    pub fn canonical_digest(&self) -> Result<String, ValueModelError> {
        if self.schema_version != 1 || self.config_version.trim().is_empty() {
            return Err(ValueModelError::InvalidConfig);
        }
        if self.ranking_criteria.len() != 9
            || self.ranking_criteria[0] != RankingCriterion::SafetyGate
            || self.ranking_criteria[1] != RankingCriterion::EvaluabilityGate
            || self.ranking_criteria[2] != RankingCriterion::EligibilityGate
            || self.ranking_criteria.last() != Some(&RankingCriterion::CandidateIdAscending)
        {
            return Err(ValueModelError::InvalidConfig);
        }
        let mut seen = std::collections::BTreeSet::new();
        if self.ranking_criteria.iter().any(|item| !seen.insert(*item)) {
            return Err(ValueModelError::InvalidConfig);
        }
        let canonical = serde_json::to_vec(self).map_err(|_| ValueModelError::InvalidConfig)?;
        let digest = Sha256::digest(canonical);
        Ok(format!(
            "sha256:{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioFactorName {
    ObservedBurden,
    EligibleFraction,
    EffectIfExposed,
    Adoption,
    UnitValueUsd,
    ImplementationCostUsd,
    OperatingCostUsd,
    Confidence,
    ImplementationEffortMinutes,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioFactors {
    pub observed_burden: FactorEstimate<u64>,
    pub eligible_fraction: FactorEstimate<Rate>,
    pub effect_if_exposed: FactorEstimate<Rate>,
    pub adoption: FactorEstimate<Rate>,
    pub unit_value_usd: FactorEstimate<UsdCents>,
    pub implementation_cost_usd: FactorEstimate<UsdCents>,
    pub operating_cost_usd: FactorEstimate<UsdCents>,
    pub confidence: FactorEstimate<Rate>,
    pub implementation_effort_minutes: FactorEstimate<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsdRange {
    pub low: UsdCents,
    pub base: UsdCents,
    pub high: UsdCents,
}

impl Serialize for UsdRange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("UsdRange", 4)?;
        state.serialize_field("currency", "USD")?;
        state.serialize_field("low", &self.low)?;
        state.serialize_field("base", &self.base)?;
        state.serialize_field("high", &self.high)?;
        state.end()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ValueAssessment {
    pub gross_usd: Option<UsdRange>,
    pub net_usd: Option<UsdRange>,
    pub unknown_factors: Vec<UnknownFactor>,
    pub interpretation: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateStatus {
    Pass,
    Fail,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RankingCandidate {
    pub candidate_id: String,
    pub scope_digest: String,
    pub safety: GateStatus,
    pub evaluability: GateStatus,
    pub eligibility: GateStatus,
    pub assessment: ValueAssessment,
    pub effort_minutes: Option<u32>,
    pub confidence: Option<Rate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioRanking {
    pub config_version: String,
    pub config_digest: String,
    pub ordered_candidate_ids: Vec<String>,
    pub deferred_candidate_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnknownFactor {
    pub factor: ScenarioFactorName,
    pub evidence_kind: EvidenceKind,
    pub reason: UnknownReason,
    pub evidence: EvidenceBinding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueModelError {
    InvalidRateRange,
    InvalidNumericRange,
    NegativeInputAmount,
    ArithmeticOverflow,
    InvalidEvidenceBinding,
    DisallowedEvidenceKind,
    HoldoutEvidenceNotAllowed,
    ObservedBurdenMustBeDiscoveryEvidence,
    InvalidConfig,
    InvalidCandidateIdentity,
}

/// Computes conservative low/base/high scenario value with whole-cent
/// round-down after each fractional factor. Gross uses corresponding factor
/// endpoints; net low subtracts the highest costs and net high the lowest.
pub fn calculate_value(factors: &ScenarioFactors) -> Result<ValueAssessment, ValueModelError> {
    validate_evidence(factors)?;
    validate_range(&factors.observed_burden, |_| Ok(()))?;
    validate_range(&factors.eligible_fraction, |rate| validate_rate(*rate))?;
    validate_range(&factors.effect_if_exposed, |rate| validate_rate(*rate))?;
    validate_range(&factors.adoption, |rate| validate_rate(*rate))?;
    validate_range(&factors.unit_value_usd, |money| {
        validate_nonnegative_money(*money)
    })?;
    validate_range(&factors.implementation_cost_usd, |money| {
        validate_nonnegative_money(*money)
    })?;
    validate_range(&factors.operating_cost_usd, |money| {
        validate_nonnegative_money(*money)
    })?;
    validate_range(&factors.confidence, |rate| validate_rate(*rate))?;
    validate_range(&factors.implementation_effort_minutes, |_| Ok(()))?;

    let mut unknown_factors = Vec::new();
    collect_unknown(
        &factors.observed_burden,
        ScenarioFactorName::ObservedBurden,
        &mut unknown_factors,
    );
    collect_unknown(
        &factors.eligible_fraction,
        ScenarioFactorName::EligibleFraction,
        &mut unknown_factors,
    );
    collect_unknown(
        &factors.effect_if_exposed,
        ScenarioFactorName::EffectIfExposed,
        &mut unknown_factors,
    );
    collect_unknown(
        &factors.adoption,
        ScenarioFactorName::Adoption,
        &mut unknown_factors,
    );
    collect_unknown(
        &factors.unit_value_usd,
        ScenarioFactorName::UnitValueUsd,
        &mut unknown_factors,
    );
    collect_unknown(
        &factors.implementation_cost_usd,
        ScenarioFactorName::ImplementationCostUsd,
        &mut unknown_factors,
    );
    collect_unknown(
        &factors.operating_cost_usd,
        ScenarioFactorName::OperatingCostUsd,
        &mut unknown_factors,
    );
    collect_unknown(
        &factors.confidence,
        ScenarioFactorName::Confidence,
        &mut unknown_factors,
    );
    collect_unknown(
        &factors.implementation_effort_minutes,
        ScenarioFactorName::ImplementationEffortMinutes,
        &mut unknown_factors,
    );
    unknown_factors.sort_by_key(|unknown| unknown.factor);

    let gross_inputs = (
        known_range(&factors.observed_burden),
        known_range(&factors.eligible_fraction),
        known_range(&factors.effect_if_exposed),
        known_range(&factors.adoption),
        known_range(&factors.unit_value_usd),
    );
    let gross_usd = match gross_inputs {
        (Some(burden), Some(eligible), Some(effect), Some(adoption), Some(unit_value)) => {
            Some(UsdRange {
                low: calculate_gross(
                    burden.low,
                    eligible.low,
                    effect.low,
                    adoption.low,
                    unit_value.low,
                )?,
                base: calculate_gross(
                    burden.base,
                    eligible.base,
                    effect.base,
                    adoption.base,
                    unit_value.base,
                )?,
                high: calculate_gross(
                    burden.high,
                    eligible.high,
                    effect.high,
                    adoption.high,
                    unit_value.high,
                )?,
            })
        }
        _ => None,
    };

    let net_usd = match (
        gross_usd,
        known_range(&factors.implementation_cost_usd),
        known_range(&factors.operating_cost_usd),
    ) {
        (Some(gross), Some(implementation), Some(operating)) => Some(UsdRange {
            low: subtract_costs(gross.low, implementation.high, operating.high)?,
            base: subtract_costs(gross.base, implementation.base, operating.base)?,
            high: subtract_costs(gross.high, implementation.low, operating.low)?,
        }),
        _ => None,
    };

    Ok(ValueAssessment {
        gross_usd,
        net_usd,
        unknown_factors,
        interpretation: "modeled_scenario_only",
    })
}

/// Orders scenarios for review. It deliberately does not aggregate values,
/// solve a budgeted portfolio, or select a top-k subset.
pub fn rank_scenarios(
    candidates: &[RankingCandidate],
    config: &ValueModelConfig,
) -> Result<ScenarioRanking, ValueModelError> {
    let config_digest = config.canonical_digest()?;
    let mut identifiers = std::collections::BTreeSet::new();
    for candidate in candidates {
        if candidate.candidate_id.trim().is_empty()
            || !is_sha256_digest(&candidate.candidate_id)
            || !identifiers.insert(candidate.candidate_id.clone())
            || !is_sha256_digest(&candidate.scope_digest)
        {
            return Err(ValueModelError::InvalidCandidateIdentity);
        }
    }
    let mut ranked = candidates.to_vec();
    ranked.sort_by(|left, right| compare_candidates(left, right, &config.ranking_criteria));
    let actionable = ranked
        .iter()
        .filter(|candidate| {
            candidate.safety == GateStatus::Pass
                && candidate.evaluability == GateStatus::Pass
                && candidate.eligibility == GateStatus::Pass
        })
        .map(|candidate| candidate.candidate_id.clone())
        .collect();
    let deferred = ranked
        .iter()
        .filter(|candidate| {
            candidate.safety != GateStatus::Pass
                || candidate.evaluability != GateStatus::Pass
                || candidate.eligibility != GateStatus::Pass
        })
        .map(|candidate| candidate.candidate_id.clone())
        .collect();
    Ok(ScenarioRanking {
        config_version: config.config_version.clone(),
        config_digest,
        ordered_candidate_ids: actionable,
        deferred_candidate_ids: deferred,
    })
}

fn is_sha256_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn compare_candidates(
    left: &RankingCandidate,
    right: &RankingCandidate,
    criteria: &[RankingCriterion],
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    for criterion in criteria {
        let ordering = match criterion {
            RankingCriterion::SafetyGate => gate_order(left.safety).cmp(&gate_order(right.safety)),
            RankingCriterion::EvaluabilityGate => {
                gate_order(left.evaluability).cmp(&gate_order(right.evaluability))
            }
            RankingCriterion::EligibilityGate => {
                gate_order(left.eligibility).cmp(&gate_order(right.eligibility))
            }
            RankingCriterion::NetBaseUsdDescending => descending_option(
                left.assessment.net_usd.map(|range| range.base.0),
                right.assessment.net_usd.map(|range| range.base.0),
            ),
            RankingCriterion::UncertaintyWidthAscending => ascending_option(
                left.assessment
                    .net_usd
                    .and_then(|range| range.high.0.checked_sub(range.low.0)),
                right
                    .assessment
                    .net_usd
                    .and_then(|range| range.high.0.checked_sub(range.low.0)),
            ),
            RankingCriterion::EffortMinutesAscending => {
                ascending_option(left.effort_minutes, right.effort_minutes)
            }
            RankingCriterion::ConfidenceDescending => descending_option(
                left.confidence.map(Rate::basis_points),
                right.confidence.map(Rate::basis_points),
            ),
            RankingCriterion::ScopeDigestAscending => left.scope_digest.cmp(&right.scope_digest),
            RankingCriterion::CandidateIdAscending => left.candidate_id.cmp(&right.candidate_id),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.candidate_id.cmp(&right.candidate_id)
}

fn gate_order(status: GateStatus) -> u8 {
    match status {
        GateStatus::Pass => 0,
        GateStatus::Unknown => 1,
        GateStatus::Fail => 2,
    }
}

fn ascending_option<T: Ord>(left: Option<T>, right: Option<T>) -> std::cmp::Ordering {
    left.is_none()
        .cmp(&right.is_none())
        .then_with(|| left.cmp(&right))
}

fn descending_option<T: Ord>(left: Option<T>, right: Option<T>) -> std::cmp::Ordering {
    left.is_none()
        .cmp(&right.is_none())
        .then_with(|| right.cmp(&left))
}

fn calculate_gross(
    burden: u64,
    eligible: Rate,
    effect: Rate,
    adoption: Rate,
    unit_value: UsdCents,
) -> Result<UsdCents, ValueModelError> {
    let mut numerator = u128::from(burden);
    let mut denominator = RATE_SCALE.pow(3);
    for rate in [eligible, effect, adoption] {
        let mut rate_numerator = u128::from(rate.0);
        let common = gcd(rate_numerator, denominator);
        rate_numerator /= common;
        denominator /= common;
        let common = gcd(numerator, denominator);
        numerator /= common;
        denominator /= common;
        numerator = numerator
            .checked_mul(rate_numerator)
            .ok_or(ValueModelError::ArithmeticOverflow)?;
    }
    let unit_value =
        u128::try_from(unit_value.0).map_err(|_| ValueModelError::NegativeInputAmount)?;
    let whole = numerator / denominator;
    let remainder = numerator % denominator;
    let cents = whole
        .checked_mul(unit_value)
        .and_then(|value| {
            remainder
                .checked_mul(unit_value)
                .and_then(|fractional| value.checked_add(fractional / denominator))
        })
        .ok_or(ValueModelError::ArithmeticOverflow)?;
    let cents = i128::try_from(cents).map_err(|_| ValueModelError::ArithmeticOverflow)?;
    Ok(UsdCents(cents))
}

fn subtract_costs(
    gross: UsdCents,
    implementation: UsdCents,
    operating: UsdCents,
) -> Result<UsdCents, ValueModelError> {
    gross
        .0
        .checked_sub(implementation.0)
        .and_then(|value| value.checked_sub(operating.0))
        .map(UsdCents)
        .ok_or(ValueModelError::ArithmeticOverflow)
}

fn validate_rate(value: Rate) -> Result<(), ValueModelError> {
    if u128::from(value.0) <= RATE_SCALE {
        Ok(())
    } else {
        Err(ValueModelError::InvalidRateRange)
    }
}

fn validate_nonnegative_money(value: UsdCents) -> Result<(), ValueModelError> {
    if value.0 >= 0 && value.0 <= i64::MAX as i128 {
        Ok(())
    } else {
        Err(ValueModelError::NegativeInputAmount)
    }
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn validate_range<T, F>(estimate: &FactorEstimate<T>, validate: F) -> Result<(), ValueModelError>
where
    T: Ord,
    F: Fn(&T) -> Result<(), ValueModelError>,
{
    if let FactorEstimate::Known { range, .. } = estimate {
        if range.low > range.base || range.base > range.high {
            return Err(ValueModelError::InvalidNumericRange);
        }
        validate(&range.low)?;
        validate(&range.base)?;
        validate(&range.high)?;
    }
    Ok(())
}

fn known_range<T>(estimate: &FactorEstimate<T>) -> Option<&FactorRange<T>> {
    match estimate {
        FactorEstimate::Known { range, .. } => Some(range),
        FactorEstimate::Unknown { .. } => None,
    }
}

fn collect_unknown<T>(
    estimate: &FactorEstimate<T>,
    name: ScenarioFactorName,
    into: &mut Vec<UnknownFactor>,
) {
    if let FactorEstimate::Unknown {
        evidence_kind,
        evidence,
        reason,
    } = estimate
    {
        into.push(UnknownFactor {
            factor: name,
            evidence_kind: *evidence_kind,
            reason: *reason,
            evidence: evidence.clone(),
        });
    }
}

fn validate_evidence(factors: &ScenarioFactors) -> Result<(), ValueModelError> {
    let estimates: [(EvidenceKind, &EvidenceBinding); 9] = [
        (
            kind(&factors.observed_burden),
            binding(&factors.observed_burden),
        ),
        (
            kind(&factors.eligible_fraction),
            binding(&factors.eligible_fraction),
        ),
        (
            kind(&factors.effect_if_exposed),
            binding(&factors.effect_if_exposed),
        ),
        (kind(&factors.adoption), binding(&factors.adoption)),
        (
            kind(&factors.unit_value_usd),
            binding(&factors.unit_value_usd),
        ),
        (
            kind(&factors.implementation_cost_usd),
            binding(&factors.implementation_cost_usd),
        ),
        (
            kind(&factors.operating_cost_usd),
            binding(&factors.operating_cost_usd),
        ),
        (kind(&factors.confidence), binding(&factors.confidence)),
        (
            kind(&factors.implementation_effort_minutes),
            binding(&factors.implementation_effort_minutes),
        ),
    ];
    for (kind, evidence) in estimates {
        if !is_sha256_digest(&evidence.run_digest)
            || !is_sha256_digest(&evidence.source_snapshot_digest)
            || !is_sha256_digest(&evidence.evidence_digest)
        {
            return Err(ValueModelError::InvalidEvidenceBinding);
        }
        if kind == EvidenceKind::FutureBank {
            return Err(ValueModelError::DisallowedEvidenceKind);
        }
        if evidence.phase == EvidencePhase::ReproductionHoldout {
            return Err(ValueModelError::HoldoutEvidenceNotAllowed);
        }
    }
    let observed_evidence = binding(&factors.observed_burden);
    for (kind, evidence) in estimates {
        if kind == EvidenceKind::SuppliedSynthetic
            && (evidence.phase != EvidencePhase::Discovery
                || evidence.run_digest != observed_evidence.run_digest
                || evidence.source_snapshot_digest != observed_evidence.source_snapshot_digest)
        {
            return Err(ValueModelError::InvalidEvidenceBinding);
        }
    }
    match (
        &factors.observed_burden,
        kind(&factors.observed_burden),
        binding(&factors.observed_burden),
    ) {
        (
            FactorEstimate::Known { .. } | FactorEstimate::Unknown { .. },
            EvidenceKind::SuppliedSynthetic,
            evidence,
        ) if evidence.phase == EvidencePhase::Discovery => Ok(()),
        _ => Err(ValueModelError::ObservedBurdenMustBeDiscoveryEvidence),
    }
}

fn kind<T>(estimate: &FactorEstimate<T>) -> EvidenceKind {
    match estimate {
        FactorEstimate::Known { evidence_kind, .. }
        | FactorEstimate::Unknown { evidence_kind, .. } => *evidence_kind,
    }
}

fn binding<T>(estimate: &FactorEstimate<T>) -> &EvidenceBinding {
    match estimate {
        FactorEstimate::Known { evidence, .. } | FactorEstimate::Unknown { evidence, .. } => {
            evidence
        }
    }
}
