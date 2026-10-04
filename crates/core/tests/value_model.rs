use improvement_engine_core::value_model::{
    EvidenceBinding, EvidenceKind, EvidencePhase, FactorEstimate, FactorRange, GateStatus,
    RankingCandidate, Rate, ScenarioFactors, UnknownReason, UsdCents, ValueModelConfig,
    ValueModelError, calculate_value, rank_scenarios,
};
use sha2::{Digest, Sha256};

fn opaque_id(label: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(label.as_bytes()))
}

fn evidence(kind: EvidenceKind) -> EvidenceBinding {
    EvidenceBinding {
        run_digest: format!("sha256:{}", "b".repeat(64)),
        source_snapshot_digest: format!("sha256:{}", "c".repeat(64)),
        evidence_digest: format!(
            "sha256:{}",
            (if kind == EvidenceKind::TeamGenerated {
                "d"
            } else {
                "e"
            })
            .repeat(64)
        ),
        phase: if kind == EvidenceKind::SuppliedSynthetic {
            EvidencePhase::Discovery
        } else {
            EvidencePhase::Other
        },
    }
}

fn known<T>(low: T, base: T, high: T, evidence_kind: EvidenceKind) -> FactorEstimate<T> {
    FactorEstimate::known(
        FactorRange { low, base, high },
        evidence_kind,
        evidence(evidence_kind),
    )
}

#[test]
fn calculates_bounded_scenario_value_with_fixed_point_arithmetic() {
    let factors = ScenarioFactors {
        observed_burden: known(100_u64, 120, 140, EvidenceKind::SuppliedSynthetic),
        eligible_fraction: known(
            Rate::from_basis_points(5_000),
            Rate::from_basis_points(6_000),
            Rate::from_basis_points(7_000),
            EvidenceKind::ExternalAssumption,
        ),
        effect_if_exposed: known(
            Rate::from_basis_points(1_000),
            Rate::from_basis_points(2_000),
            Rate::from_basis_points(3_000),
            EvidenceKind::ExternalAssumption,
        ),
        adoption: known(
            Rate::from_basis_points(4_000),
            Rate::from_basis_points(5_000),
            Rate::from_basis_points(6_000),
            EvidenceKind::ExternalAssumption,
        ),
        unit_value_usd: known(
            UsdCents::new(10_000),
            UsdCents::new(12_000),
            UsdCents::new(15_000),
            EvidenceKind::ExternalAssumption,
        ),
        implementation_cost_usd: known(
            UsdCents::new(50_000),
            UsdCents::new(60_000),
            UsdCents::new(70_000),
            EvidenceKind::ExternalAssumption,
        ),
        operating_cost_usd: known(
            UsdCents::new(5_000),
            UsdCents::new(6_000),
            UsdCents::new(7_000),
            EvidenceKind::ExternalAssumption,
        ),
        confidence: known(
            Rate::from_basis_points(6_000),
            Rate::from_basis_points(7_000),
            Rate::from_basis_points(8_000),
            EvidenceKind::TeamGenerated,
        ),
        implementation_effort_minutes: known(60_u32, 90, 120, EvidenceKind::TeamGenerated),
    };

    let result = calculate_value(&factors).expect("valid scenario factors");

    let gross = result.gross_usd.expect("all gross factors are supported");
    let net = result.net_usd.expect("all net factors are supported");
    assert_eq!(gross.low, UsdCents::new(20_000));
    assert_eq!(gross.base, UsdCents::new(86_400));
    assert_eq!(gross.high, UsdCents::new(264_600));
    assert_eq!(net.low, UsdCents::new(-57_000));
    assert_eq!(net.base, UsdCents::new(20_400));
    assert_eq!(net.high, UsdCents::new(209_600));
    assert_eq!(result.interpretation, "modeled_scenario_only");
}

fn baseline() -> ScenarioFactors {
    ScenarioFactors {
        observed_burden: known(100_u64, 120, 140, EvidenceKind::SuppliedSynthetic),
        eligible_fraction: known(
            Rate::from_basis_points(5_000),
            Rate::from_basis_points(6_000),
            Rate::from_basis_points(7_000),
            EvidenceKind::ExternalAssumption,
        ),
        effect_if_exposed: known(
            Rate::from_basis_points(1_000),
            Rate::from_basis_points(2_000),
            Rate::from_basis_points(3_000),
            EvidenceKind::ExternalAssumption,
        ),
        adoption: known(
            Rate::from_basis_points(4_000),
            Rate::from_basis_points(5_000),
            Rate::from_basis_points(6_000),
            EvidenceKind::ExternalAssumption,
        ),
        unit_value_usd: known(
            UsdCents::new(10_000),
            UsdCents::new(12_000),
            UsdCents::new(15_000),
            EvidenceKind::ExternalAssumption,
        ),
        implementation_cost_usd: known(
            UsdCents::new(50_000),
            UsdCents::new(60_000),
            UsdCents::new(70_000),
            EvidenceKind::ExternalAssumption,
        ),
        operating_cost_usd: known(
            UsdCents::new(5_000),
            UsdCents::new(6_000),
            UsdCents::new(7_000),
            EvidenceKind::ExternalAssumption,
        ),
        confidence: known(
            Rate::from_basis_points(6_000),
            Rate::from_basis_points(7_000),
            Rate::from_basis_points(8_000),
            EvidenceKind::TeamGenerated,
        ),
        implementation_effort_minutes: known(60_u32, 90, 120, EvidenceKind::TeamGenerated),
    }
}

#[test]
fn unknown_unit_value_keeps_gross_and_net_unknown_with_reason() {
    let mut factors = baseline();
    factors.unit_value_usd = FactorEstimate::unknown(
        EvidenceKind::ExternalAssumption,
        UnknownReason::NoSupportedUsdValue,
        evidence(EvidenceKind::ExternalAssumption),
    );

    let assessment = calculate_value(&factors).expect("unknown is not an invalid number");

    assert_eq!(assessment.gross_usd, None);
    assert_eq!(assessment.net_usd, None);
    assert_eq!(assessment.unknown_factors.len(), 1);
    assert_eq!(
        assessment.unknown_factors[0].reason,
        UnknownReason::NoSupportedUsdValue
    );
}

#[test]
fn unknown_cost_preserves_gross_but_does_not_fabricate_net() {
    let mut factors = baseline();
    factors.implementation_cost_usd = FactorEstimate::unknown(
        EvidenceKind::ExternalAssumption,
        UnknownReason::NotMeasured,
        evidence(EvidenceKind::ExternalAssumption),
    );

    let assessment = calculate_value(&factors).expect("unknown cost is allowed");

    assert!(assessment.gross_usd.is_some());
    assert_eq!(assessment.net_usd, None);
    assert_eq!(assessment.unknown_factors.len(), 1);
    assert_eq!(
        assessment.unknown_factors[0].reason,
        UnknownReason::NotMeasured
    );
}

#[test]
fn unknown_confidence_and_effort_are_retained_for_ranking() {
    let mut factors = baseline();
    factors.confidence = FactorEstimate::unknown(
        EvidenceKind::TeamGenerated,
        UnknownReason::NotMeasured,
        evidence(EvidenceKind::TeamGenerated),
    );
    factors.implementation_effort_minutes = FactorEstimate::unknown(
        EvidenceKind::TeamGenerated,
        UnknownReason::NotMeasured,
        evidence(EvidenceKind::TeamGenerated),
    );

    let assessment =
        calculate_value(&factors).expect("these factors do not erase modeled gross value");

    assert!(assessment.gross_usd.is_some());
    assert!(
        assessment.unknown_factors.iter().any(|item| item.factor
            == improvement_engine_core::value_model::ScenarioFactorName::Confidence)
    );
    assert!(assessment.unknown_factors.iter().any(|item| item.factor
        == improvement_engine_core::value_model::ScenarioFactorName::ImplementationEffortMinutes));
}

#[test]
fn rejects_rate_above_one_hundred_percent_and_invalid_ranges() {
    let mut factors = baseline();
    factors.adoption = known(
        Rate::from_basis_points(0),
        Rate::from_basis_points(10_001),
        Rate::from_basis_points(10_001),
        EvidenceKind::ExternalAssumption,
    );
    assert_eq!(
        calculate_value(&factors),
        Err(improvement_engine_core::value_model::ValueModelError::InvalidRateRange)
    );

    let mut factors = baseline();
    factors.observed_burden = known(3_u64, 2, 4, EvidenceKind::SuppliedSynthetic);
    assert_eq!(
        calculate_value(&factors),
        Err(improvement_engine_core::value_model::ValueModelError::InvalidNumericRange)
    );

    let mut factors = baseline();
    factors.operating_cost_usd = known(
        UsdCents::new(-1),
        UsdCents::new(0),
        UsdCents::new(1),
        EvidenceKind::ExternalAssumption,
    );
    assert_eq!(
        calculate_value(&factors),
        Err(improvement_engine_core::value_model::ValueModelError::NegativeInputAmount)
    );
}

#[test]
fn serializes_money_as_decimal_strings_with_currency_and_unknown_reasons() {
    let mut factors = baseline();
    factors.observed_burden = FactorEstimate::unknown(
        EvidenceKind::SuppliedSynthetic,
        UnknownReason::InsufficientPopulationEvidence,
        evidence(EvidenceKind::SuppliedSynthetic),
    );
    let assessment = calculate_value(&factors).expect("unknown volume should be represented");
    let json = serde_json::to_value(&assessment).expect("serializable assessment");
    assert_eq!(json["gross_usd"], serde_json::Value::Null);
    assert_eq!(json["net_usd"], serde_json::Value::Null);
    assert_eq!(
        json["unknown_factors"][0]["reason"],
        "insufficient_population_evidence"
    );
    assert_eq!(
        serde_json::to_string(&UsdCents::new(-123_456)).unwrap(),
        "\"-1234.56\""
    );
    let money = improvement_engine_core::value_model::UsdRange {
        low: UsdCents::new(100),
        base: UsdCents::new(200),
        high: UsdCents::new(300),
    };
    let money_json = serde_json::to_value(money).unwrap();
    assert_eq!(money_json["currency"], "USD");
    assert_eq!(money_json["low"], "1.00");
}

fn assess(factors: &ScenarioFactors) -> improvement_engine_core::value_model::ValueAssessment {
    calculate_value(factors).expect("valid evidence-bound factors")
}

fn candidate(id: &str, safety: GateStatus, net: i128) -> RankingCandidate {
    let mut result = assess(&baseline());
    result.net_usd = Some(improvement_engine_core::value_model::UsdRange {
        low: UsdCents::new(net),
        base: UsdCents::new(net),
        high: UsdCents::new(net),
    });
    RankingCandidate {
        candidate_id: opaque_id(id),
        scope_digest: format!("sha256:{}", "a".repeat(64)),
        safety,
        evaluability: GateStatus::Pass,
        eligibility: GateStatus::Pass,
        assessment: result,
        effort_minutes: Some(10),
        confidence: Some(Rate::from_basis_points(8_000)),
    }
}

#[test]
fn versioned_config_digest_is_stable_sensitive_and_gates_must_lead() {
    let config = ValueModelConfig::default();
    assert_eq!(
        config.canonical_digest().unwrap(),
        config.canonical_digest().unwrap()
    );
    let mut changed = config.clone();
    changed.config_version = "value-model-2".to_owned();
    assert_ne!(
        config.canonical_digest().unwrap(),
        changed.canonical_digest().unwrap()
    );
    let mut invalid = config;
    invalid.ranking_criteria.swap(0, 3);
    assert_eq!(
        invalid.canonical_digest(),
        Err(ValueModelError::InvalidConfig)
    );
}

#[test]
fn ranking_puts_safety_and_evaluability_before_value_and_is_deterministic() {
    let unsafe_high = candidate("unsafe-high", GateStatus::Fail, 99_000_000);
    let safe_low = candidate("safe-low", GateStatus::Pass, 100);
    let ranked = rank_scenarios(&[unsafe_high, safe_low], &ValueModelConfig::default()).unwrap();
    assert_eq!(ranked.ordered_candidate_ids, [opaque_id("safe-low")]);
    assert_eq!(ranked.deferred_candidate_ids, [opaque_id("unsafe-high")]);
    assert!(ranked.config_digest.starts_with("sha256:"));
}

#[test]
fn scenario_ranking_rejects_empty_or_duplicate_ids_and_requires_digest_scope() {
    let mut duplicate = candidate("same", GateStatus::Pass, 100);
    assert_eq!(
        rank_scenarios(
            &[duplicate.clone(), duplicate.clone()],
            &ValueModelConfig::default()
        ),
        Err(ValueModelError::InvalidCandidateIdentity)
    );
    duplicate.scope_digest = "account-123".to_owned();
    assert_eq!(
        rank_scenarios(&[duplicate], &ValueModelConfig::default()),
        Err(ValueModelError::InvalidCandidateIdentity)
    );
}

#[test]
fn ranking_orders_base_value_then_uncertainty_and_defers_unknown_gates() {
    let mut wider = candidate("wider", GateStatus::Pass, 500);
    wider.assessment.net_usd = Some(improvement_engine_core::value_model::UsdRange {
        low: UsdCents::new(100),
        base: UsdCents::new(500),
        high: UsdCents::new(900),
    });
    let mut narrower = candidate("narrower", GateStatus::Pass, 500);
    narrower.assessment.net_usd = Some(improvement_engine_core::value_model::UsdRange {
        low: UsdCents::new(450),
        base: UsdCents::new(500),
        high: UsdCents::new(550),
    });
    let mut unknown_eval = candidate("unknown-eval", GateStatus::Pass, 5_000);
    unknown_eval.evaluability = GateStatus::Unknown;

    let ranked = rank_scenarios(
        &[wider, unknown_eval, narrower],
        &ValueModelConfig::default(),
    )
    .unwrap();
    assert_eq!(
        ranked.ordered_candidate_ids,
        [opaque_id("narrower"), opaque_id("wider")]
    );
    assert_eq!(ranked.deferred_candidate_ids, [opaque_id("unknown-eval")]);
}

#[test]
fn ranking_rejects_raw_candidate_identifiers_that_may_contain_pii() {
    let mut candidate = candidate("safe-id", GateStatus::Pass, 100);
    candidate.candidate_id = "customer-12345678".to_owned();
    assert_eq!(
        rank_scenarios(&[candidate], &ValueModelConfig::default()),
        Err(ValueModelError::InvalidCandidateIdentity)
    );
}

#[test]
fn e0_valuation_rejects_future_bank_and_post_selection_holdout_evidence() {
    let mut factors = baseline();
    factors.effect_if_exposed = known(
        Rate::from_basis_points(100),
        Rate::from_basis_points(200),
        Rate::from_basis_points(300),
        EvidenceKind::FutureBank,
    );
    assert_eq!(
        calculate_value(&factors),
        Err(ValueModelError::DisallowedEvidenceKind)
    );

    let mut factors = baseline();
    let mut holdout = evidence(EvidenceKind::SuppliedSynthetic);
    holdout.phase = EvidencePhase::ReproductionHoldout;
    factors.observed_burden = FactorEstimate::known(
        FactorRange {
            low: 100,
            base: 120,
            high: 140,
        },
        EvidenceKind::SuppliedSynthetic,
        holdout,
    );
    assert_eq!(
        calculate_value(&factors),
        Err(ValueModelError::HoldoutEvidenceNotAllowed)
    );
}

#[test]
fn supplied_synthetic_factors_must_share_the_observed_discovery_run_and_snapshot() {
    let mut factors = baseline();
    let mut other_discovery = evidence(EvidenceKind::SuppliedSynthetic);
    other_discovery.evidence_digest = format!("sha256:{}", "f".repeat(64));
    other_discovery.run_digest = format!("sha256:{}", "9".repeat(64));
    factors.eligible_fraction = FactorEstimate::known(
        FactorRange {
            low: Rate::from_basis_points(1_000),
            base: Rate::from_basis_points(2_000),
            high: Rate::from_basis_points(3_000),
        },
        EvidenceKind::SuppliedSynthetic,
        other_discovery,
    );
    assert_eq!(
        calculate_value(&factors),
        Err(ValueModelError::InvalidEvidenceBinding)
    );
}

#[test]
fn unknown_factor_serde_round_trips_and_rejects_extra_fields() {
    let unknown = FactorEstimate::<u64>::unknown(
        EvidenceKind::SuppliedSynthetic,
        UnknownReason::NotMeasured,
        evidence(EvidenceKind::SuppliedSynthetic),
    );
    let encoded = serde_json::to_string(&unknown).unwrap();
    let decoded: FactorEstimate<u64> = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, unknown);
    let mut with_extra = serde_json::to_value(&unknown).unwrap();
    with_extra
        .as_object_mut()
        .unwrap()
        .insert("extra".to_owned(), serde_json::Value::Bool(true));
    assert!(serde_json::from_value::<FactorEstimate<u64>>(with_extra).is_err());
}

#[test]
fn evidence_and_value_wire_shapes_retain_provenance_and_signed_net() {
    let mut factors = baseline();
    factors.unit_value_usd = known(
        UsdCents::new(1),
        UsdCents::new(1),
        UsdCents::new(1),
        EvidenceKind::ExternalAssumption,
    );
    factors.implementation_cost_usd = known(
        UsdCents::new(10_000),
        UsdCents::new(10_000),
        UsdCents::new(10_000),
        EvidenceKind::ExternalAssumption,
    );
    factors.operating_cost_usd = known(
        UsdCents::new(0),
        UsdCents::new(0),
        UsdCents::new(0),
        EvidenceKind::ExternalAssumption,
    );
    let assessment = assess(&factors);
    let json = serde_json::to_value(&assessment).unwrap();
    assert!(json["net_usd"]["base"].as_str().unwrap().starts_with('-'));
    assert_eq!(json["gross_usd"]["currency"], "USD");
    assert_eq!(json["unknown_factors"], serde_json::json!([]));
}

#[test]
fn rounds_down_at_cent_boundary_and_handles_bounded_i128_sized_products() {
    let mut factors = baseline();
    factors.observed_burden = known(1_u64, 1, 1, EvidenceKind::SuppliedSynthetic);
    factors.eligible_fraction = known(
        Rate::from_basis_points(5_000),
        Rate::from_basis_points(5_000),
        Rate::from_basis_points(5_000),
        EvidenceKind::ExternalAssumption,
    );
    factors.effect_if_exposed = factors.eligible_fraction.clone();
    factors.adoption = factors.eligible_fraction.clone();
    factors.unit_value_usd = known(
        UsdCents::new(1),
        UsdCents::new(1),
        UsdCents::new(1),
        EvidenceKind::ExternalAssumption,
    );
    let rounded = assess(&factors);
    assert_eq!(rounded.gross_usd.unwrap().base, UsdCents::new(0));

    let mut maximum = baseline();
    maximum.observed_burden = known(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        EvidenceKind::SuppliedSynthetic,
    );
    maximum.eligible_fraction = known(
        Rate::from_basis_points(10_000),
        Rate::from_basis_points(10_000),
        Rate::from_basis_points(10_000),
        EvidenceKind::ExternalAssumption,
    );
    maximum.effect_if_exposed = maximum.eligible_fraction.clone();
    maximum.adoption = maximum.eligible_fraction.clone();
    maximum.unit_value_usd = known(
        UsdCents::new(i64::MAX as i128),
        UsdCents::new(i64::MAX as i128),
        UsdCents::new(i64::MAX as i128),
        EvidenceKind::ExternalAssumption,
    );
    maximum.implementation_cost_usd = known(
        UsdCents::new(0),
        UsdCents::new(0),
        UsdCents::new(0),
        EvidenceKind::ExternalAssumption,
    );
    maximum.operating_cost_usd = maximum.implementation_cost_usd.clone();
    assert!(assess(&maximum).gross_usd.unwrap().base.cents() > 0);
}
