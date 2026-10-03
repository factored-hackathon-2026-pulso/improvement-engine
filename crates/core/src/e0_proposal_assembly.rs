//! Deterministic, simulator-only proposal candidates assembled from measured
//! E0 signal portfolio entries. This is not native Agent Core authoring.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::ArtifactReference;
use crate::local_simulation::{
    LocalRunResult, LocalSourceKind, SignalSummary, signal_summary_commitment,
};

/// The bounded result of assembling descriptive proposal inputs from one run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalProposalAssembly {
    pub status: LocalProposalAssemblyStatus,
    pub source_family: String,
    pub authority: String,
    pub source_run_id: String,
    pub source_snapshot_ref: LocalProposalSnapshotReference,
    pub observed_cutoff_rfc3339: String,
    pub primary_signal_digest: Option<String>,
    pub dispositions: Vec<LocalProposalDisposition>,
    pub candidates: Vec<LocalProposalCandidate>,
    /// Deliberately absent: no business outcome oracle or lift is claimed here.
    pub business_lift: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalProposalAssemblyStatus {
    CandidatesReady,
    InsufficientEvidence,
    NoQualifyingSignals,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalProposalDisposition {
    pub metric_id: String,
    pub signal_digest: Option<String>,
    pub summary_commitment: Option<String>,
    pub state: String,
    pub reason: String,
}

/// A proposal seed, not a ready-to-apply Agent Core artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalProposalCandidate {
    pub proposal_ref: String,
    pub source_run_id: String,
    pub source_snapshot_ref: LocalProposalSnapshotReference,
    pub observed_cutoff_rfc3339: String,
    pub metric_id: String,
    pub signal_digest: String,
    pub summary_commitment: String,
    pub detector_policy_id: String,
    pub detector_policy_version: u16,
    pub hypothesis: String,
    pub claim_level: String,
    pub route_status: String,
    pub evaluation_status: String,
    pub business_lift: Option<String>,
    pub native_agent_core_status: String,
}

/// Public-safe snapshot lineage. Tenant identifiers are deliberately omitted:
/// the source CLI permits arbitrary tenant strings up to 128 bytes, so they
/// cannot be assumed non-PII for serialized simulator output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalProposalSnapshotReference {
    pub id: String,
    pub revision: u64,
    pub digest: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalProposalAssemblyError {
    InvalidPortfolioEvidence,
}

/// Build stable, descriptive proposal seeds from every independently
/// qualifying signal in one E0 simulator run. No business-value estimate,
/// route link, Agent Core eligibility, or evaluation result is inferred.
pub fn assemble_e0_proposals(
    result: &LocalRunResult,
) -> Result<LocalProposalAssembly, LocalProposalAssemblyError> {
    let Some(portfolio) = result.local_simulation_portfolio.as_ref() else {
        return Ok(unsupported_assembly(result));
    };
    if result.source_kind != LocalSourceKind::E0
        || portfolio.source_family != "e0"
        || portfolio.authority != "simulator_only"
    {
        return Ok(unsupported_assembly(result));
    }
    if result.snapshot_ref.tenant_id != result.tenant_id {
        return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
    }
    if !valid_run_envelope(result) {
        return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
    }

    let mut signals_by_metric = BTreeMap::<&str, &SignalSummary>::new();
    let mut signal_digests = BTreeSet::new();
    for signal in &result.signals {
        if !is_e0_metric(&signal.metric_id)
            || !is_policy_code(&signal.detector_policy_id)
            || signal.detector_policy_version == 0
            || !is_digest(&signal.digest)
            || signal.summary_commitment.is_empty()
            || signal_summary_commitment(signal) != signal.summary_commitment
            || !valid_metric_counts(signal)
            || signals_by_metric
                .insert(&signal.metric_id, signal)
                .is_some()
            || !signal_digests.insert(signal.digest.as_str())
        {
            return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
        }
    }

    let mut seen_metrics = BTreeSet::new();
    let mut dispositions = Vec::with_capacity(portfolio.dispositions.len());
    let mut candidates = Vec::new();
    let mut candidate_digests = Vec::new();
    for disposition in &portfolio.dispositions {
        if !is_e0_metric(&disposition.metric_id)
            || !seen_metrics.insert(disposition.metric_id.as_str())
            || !is_allowed_reason(
                &disposition.metric_id,
                &disposition.state,
                &disposition.reason,
            )
        {
            return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
        }
        let signal = match disposition.state.as_str() {
            "candidate_for_simulated_investigation" | "not_qualified" | "insufficient_evidence" => {
                let Some(signal) = signals_by_metric
                    .get(disposition.metric_id.as_str())
                    .copied()
                else {
                    return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
                };
                if disposition.signal_digest.as_deref() != Some(signal.digest.as_str()) {
                    return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
                }
                let (expected_state, expected_reason) = expected_disposition(signal);
                if disposition.state != expected_state || disposition.reason != expected_reason {
                    return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
                }
                Some(signal)
            }
            "unavailable" => {
                if disposition.metric_id != "e0_recurring_copilot_query_cases"
                    || disposition.signal_digest.is_some()
                    || signals_by_metric.contains_key(disposition.metric_id.as_str())
                    || disposition.reason != "source_table_unavailable"
                    || result.recurrence_measurement_status != "source_table_unavailable"
                {
                    return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
                }
                None
            }
            _ => return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence),
        };
        if let Some(signal) = signal {
            if disposition.state == "candidate_for_simulated_investigation" {
                candidate_digests.push(signal.digest.clone());
                candidates.push(candidate_from_signal(result, signal));
            }
        }
        dispositions.push(LocalProposalDisposition {
            metric_id: disposition.metric_id.clone(),
            signal_digest: disposition.signal_digest.clone(),
            summary_commitment: signal.map(|item| item.summary_commitment.clone()),
            state: disposition.state.clone(),
            reason: disposition.reason.clone(),
        });
    }

    // The E0 detector has a closed three-metric contract: technical errors
    // and retries are always measured; recurring-query coverage is measured
    // or explicitly unavailable. Empty portfolios remain insufficient rather
    // than being backfilled with fabricated zero observations.
    if !portfolio.dispositions.is_empty() {
        let expected_metrics = BTreeSet::from([
            "e0_technical_error_rate",
            "e0_tool_retry_case_rate",
            "e0_recurring_copilot_query_cases",
        ]);
        if seen_metrics != expected_metrics
            || (result.recurrence_measurement_status == "observed"
                && !signals_by_metric.contains_key("e0_recurring_copilot_query_cases"))
            || (result.recurrence_measurement_status == "source_table_unavailable"
                && !portfolio.dispositions.iter().any(|item| {
                    item.metric_id == "e0_recurring_copilot_query_cases"
                        && item.state == "unavailable"
                }))
            || !matches!(
                result.recurrence_measurement_status.as_str(),
                "observed" | "source_table_unavailable"
            )
        {
            return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
        }
    }

    let disposition_signal_metrics = dispositions
        .iter()
        .filter(|item| item.signal_digest.is_some())
        .map(|item| item.metric_id.as_str())
        .collect::<BTreeSet<_>>();
    let measured_signal_metrics = signals_by_metric.keys().copied().collect::<BTreeSet<_>>();
    if disposition_signal_metrics != measured_signal_metrics
        || candidate_digests.iter().collect::<BTreeSet<_>>().len() != candidate_digests.len()
        || !same_digest_set(&candidate_digests, &portfolio.candidate_signal_digests)
        || portfolio.primary_signal_digest.as_deref()
            != result.signal.as_ref().map(|signal| signal.digest.as_str())
        || portfolio
            .primary_signal_digest
            .as_deref()
            .is_some_and(|digest| !signal_digests.contains(digest))
    {
        return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
    }

    let expected_portfolio_status = if !candidates.is_empty() {
        "candidates_ready"
    } else if dispositions.is_empty()
        || dispositions
            .iter()
            .any(|item| matches!(item.state.as_str(), "insufficient_evidence" | "unavailable"))
    {
        "insufficient_evidence"
    } else {
        "no_qualifying_signals"
    };
    if portfolio.status != expected_portfolio_status {
        return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
    }

    candidates.sort_by(|left, right| {
        left.metric_id
            .cmp(&right.metric_id)
            .then_with(|| left.signal_digest.cmp(&right.signal_digest))
    });
    dispositions.sort_by(|left, right| left.metric_id.cmp(&right.metric_id));
    let status = if !candidates.is_empty() {
        LocalProposalAssemblyStatus::CandidatesReady
    } else if portfolio.status == "insufficient_evidence" {
        LocalProposalAssemblyStatus::InsufficientEvidence
    } else if portfolio.status == "no_qualifying_signals" {
        LocalProposalAssemblyStatus::NoQualifyingSignals
    } else {
        return Err(LocalProposalAssemblyError::InvalidPortfolioEvidence);
    };
    Ok(LocalProposalAssembly {
        status,
        source_family: "e0".into(),
        authority: "simulator_only".into(),
        source_run_id: result.run_id.clone(),
        source_snapshot_ref: safe_snapshot_ref(&result.snapshot_ref),
        observed_cutoff_rfc3339: result.observed_cutoff_rfc3339.clone(),
        primary_signal_digest: portfolio.primary_signal_digest.clone(),
        dispositions,
        candidates,
        business_lift: None,
    })
}

fn unsupported_assembly(result: &LocalRunResult) -> LocalProposalAssembly {
    LocalProposalAssembly {
        status: LocalProposalAssemblyStatus::Unsupported,
        source_family: match result.source_kind {
            LocalSourceKind::E0 => "e0",
            LocalSourceKind::OriginalBank => "original_bank",
        }
        .into(),
        authority: "none".into(),
        source_run_id: "unsupported".into(),
        source_snapshot_ref: LocalProposalSnapshotReference {
            id: "unsupported".into(),
            revision: 0,
            digest: String::new(),
        },
        observed_cutoff_rfc3339: String::new(),
        primary_signal_digest: None,
        dispositions: Vec::new(),
        candidates: Vec::new(),
        business_lift: None,
    }
}

fn candidate_from_signal(
    result: &LocalRunResult,
    signal: &SignalSummary,
) -> LocalProposalCandidate {
    let hypothesis = match signal.metric_id.as_str() {
        "e0_technical_error_rate" => {
            "Investigate whether this measured technical-error pattern maps to a supported change; it is descriptive, not causal."
        }
        "e0_tool_retry_case_rate" => {
            "Investigate whether observed tool retries map to a supported flow improvement; retries alone do not establish customer harm or savings."
        }
        "e0_recurring_copilot_query_cases" => {
            "Investigate whether this recurring query pattern maps to a supported capability; recurrence alone does not establish friction or benefit."
        }
        _ => "Unsupported metric.",
    };
    LocalProposalCandidate {
        proposal_ref: format!("{}:{}", result.run_id, signal.digest),
        source_run_id: result.run_id.clone(),
        source_snapshot_ref: safe_snapshot_ref(&result.snapshot_ref),
        observed_cutoff_rfc3339: result.observed_cutoff_rfc3339.clone(),
        metric_id: signal.metric_id.clone(),
        signal_digest: signal.digest.clone(),
        summary_commitment: signal.summary_commitment.clone(),
        detector_policy_id: signal.detector_policy_id.clone(),
        detector_policy_version: signal.detector_policy_version,
        hypothesis: hypothesis.into(),
        claim_level: "descriptive_only".into(),
        route_status: "unlinked".into(),
        evaluation_status: "not_evaluated".into(),
        business_lift: None,
        native_agent_core_status: "not_connected".into(),
    }
}

fn is_e0_metric(metric_id: &str) -> bool {
    matches!(
        metric_id,
        "e0_technical_error_rate" | "e0_tool_retry_case_rate" | "e0_recurring_copilot_query_cases"
    )
}

fn is_policy_code(policy_id: &str) -> bool {
    !policy_id.is_empty()
        && policy_id.len() <= 128
        && policy_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_run_envelope(result: &LocalRunResult) -> bool {
    result.execution_mode == "local_simulation"
        && matches!(
            result.terminal_status.as_str(),
            "complete_simulated" | "complete_no_opportunity"
        )
        && result.determinism == "deterministic_given_identical_run_input"
        && is_run_id(&result.run_id)
        && !result.tenant_id.trim().is_empty()
        && result.tenant_id.len() <= 128
        && is_uuid_v7(&result.snapshot_ref.id)
        && !result.snapshot_ref.tenant_id.trim().is_empty()
        && result.snapshot_ref.tenant_id.len() <= 128
        && result.snapshot_ref.revision > 0
        && is_digest(&result.snapshot_ref.digest)
        && is_digest(&result.manifest_digest)
        && is_utc_whole_second(&result.observed_cutoff_rfc3339)
}

fn is_run_id(value: &str) -> bool {
    if value.len() > 54 {
        return false;
    }
    let Some(rest) = value.strip_prefix("run_") else {
        return false;
    };
    let Some((pid, nanos)) = rest.split_once('_') else {
        return false;
    };
    (1..=10).contains(&pid.len())
        && (1..=39).contains(&nanos.len())
        && pid
            .parse::<u32>()
            .is_ok_and(|parsed| parsed > 0 && parsed.to_string() == pid)
        && nanos
            .parse::<u128>()
            .is_ok_and(|parsed| parsed.to_string() == nanos)
}

fn is_uuid_v7(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes[14] == b'7'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        && bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index)
                || byte.is_ascii_digit()
                || (b'a'..=b'f').contains(byte)
        })
}

fn safe_snapshot_ref(reference: &ArtifactReference) -> LocalProposalSnapshotReference {
    LocalProposalSnapshotReference {
        id: reference.id.clone(),
        revision: reference.revision,
        digest: reference.digest.clone(),
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn is_utc_whole_second(value: &str) -> bool {
    if value.len() != 20
        || !value.is_ascii()
        || &value[4..5] != "-"
        || &value[7..8] != "-"
        || &value[10..11] != "T"
        || &value[13..14] != ":"
        || &value[16..17] != ":"
        || &value[19..20] != "Z"
        || !value.bytes().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        })
    {
        return false;
    }
    let year = value[0..4].parse::<u16>().ok();
    let month = value[5..7].parse::<u8>().ok();
    let day = value[8..10].parse::<u8>().ok();
    let hour = value[11..13].parse::<u8>().ok();
    let minute = value[14..16].parse::<u8>().ok();
    let second = value[17..19].parse::<u8>().ok();
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) =
        (year, month, day, hour, minute, second)
    else {
        return false;
    };
    let month_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => return false,
    };
    (1..=month_days).contains(&day) && hour < 24 && minute < 60 && second < 60
}

fn expected_disposition(signal: &SignalSummary) -> (&'static str, &'static str) {
    if signal.denominator == 0 {
        return ("insufficient_evidence", "no_known_denominator");
    }
    let qualifies = match signal.metric_id.as_str() {
        "e0_technical_error_rate" | "e0_tool_retry_case_rate" => signal.numerator > 0,
        "e0_recurring_copilot_query_cases" => {
            signal.minimum_support > 0 && signal.numerator >= signal.minimum_support
        }
        _ => false,
    };
    if qualifies {
        (
            "candidate_for_simulated_investigation",
            if signal.missing > 0 {
                "measured_threshold_met_with_missing_observations"
            } else {
                "measured_threshold_met"
            },
        )
    } else if signal.missing > 0 {
        ("insufficient_evidence", "partial_missing_observations")
    } else if signal.metric_id == "e0_recurring_copilot_query_cases" {
        ("not_qualified", "below_minimum_distinct_case_support")
    } else {
        ("not_qualified", "no_positive_measured_observation")
    }
}

fn valid_metric_counts(signal: &SignalSummary) -> bool {
    if signal.numerator > signal.denominator {
        return false;
    }
    let Some(total) = signal.denominator.checked_add(signal.missing) else {
        return false;
    };
    let Some(scaled_denominator) = signal.denominator.checked_mul(10_000) else {
        return false;
    };
    let expected_coverage = scaled_denominator.checked_div(total).unwrap_or_default() as u16;
    signal.coverage_basis_points == expected_coverage
}

fn is_allowed_reason(metric_id: &str, state: &str, reason: &str) -> bool {
    matches!(
        (metric_id, state, reason),
        (
            _,
            "candidate_for_simulated_investigation",
            "measured_threshold_met"
        ) | (
            _,
            "candidate_for_simulated_investigation",
            "measured_threshold_met_with_missing_observations",
        ) | (_, "insufficient_evidence", "no_known_denominator")
            | (_, "insufficient_evidence", "partial_missing_observations")
            | (_, "not_qualified", "no_positive_measured_observation")
            | (
                "e0_recurring_copilot_query_cases",
                "not_qualified",
                "below_minimum_distinct_case_support",
            )
            | (
                "e0_recurring_copilot_query_cases",
                "unavailable",
                "source_table_unavailable"
            )
    )
}

fn same_digest_set(left: &[String], right: &[String]) -> bool {
    left.len() == right.len()
        && left.iter().collect::<BTreeSet<_>>() == right.iter().collect::<BTreeSet<_>>()
}

#[cfg(test)]
mod tests {
    use crate::{
        ArtifactReference,
        local_simulation::{
            LocalRunResult, LocalSimulationPortfolio, LocalSimulationSignalDisposition,
            LocalSourceKind, SignalSummary, signal_summary_commitment,
        },
    };

    use super::assemble_e0_proposals;

    fn signal(metric_id: &str, digest: &str, numerator: u64) -> SignalSummary {
        let mut signal = SignalSummary {
            metric_id: metric_id.into(),
            detector_policy_id: format!("{metric_id}_policy"),
            detector_policy_version: 1,
            minimum_support: 20,
            numerator,
            denominator: 100,
            missing: 0,
            coverage_basis_points: 10_000,
            pattern_ref: None,
            digest: digest.into(),
            summary_commitment: String::new(),
        };
        signal.summary_commitment = signal_summary_commitment(&signal);
        signal
    }

    fn result_with(
        mut signals: Vec<SignalSummary>,
        mut dispositions: Vec<LocalSimulationSignalDisposition>,
        candidate_digests: Vec<String>,
        portfolio_status: &str,
    ) -> LocalRunResult {
        let recurrence_unavailable = dispositions.iter().any(|item| {
            item.metric_id == "e0_recurring_copilot_query_cases" && item.state == "unavailable"
        });
        if !signals.is_empty() {
            for metric_id in [
                "e0_technical_error_rate",
                "e0_tool_retry_case_rate",
                "e0_recurring_copilot_query_cases",
            ] {
                if metric_id == "e0_recurring_copilot_query_cases" && recurrence_unavailable {
                    continue;
                }
                if !signals.iter().any(|signal| signal.metric_id == metric_id) {
                    let marker = match metric_id {
                        "e0_technical_error_rate" => 'c',
                        "e0_tool_retry_case_rate" => 'd',
                        _ => 'e',
                    };
                    let digest = format!("sha256:{}", marker.to_string().repeat(64));
                    let mut placeholder = signal(metric_id, &digest, 0);
                    placeholder.detector_policy_id = format!("{metric_id}_v1");
                    placeholder.summary_commitment = signal_summary_commitment(&placeholder);
                    signals.push(placeholder.clone());
                    dispositions.push(LocalSimulationSignalDisposition {
                        metric_id: metric_id.into(),
                        signal_digest: Some(placeholder.digest),
                        state: "not_qualified".into(),
                        reason: if metric_id == "e0_recurring_copilot_query_cases" {
                            "below_minimum_distinct_case_support"
                        } else {
                            "no_positive_measured_observation"
                        }
                        .into(),
                    });
                }
            }
        }
        let primary = signals.first().cloned();
        LocalRunResult {
            run_id: "run_42_1770000000000000000".into(),
            tenant_id: "pulso_local".into(),
            source_kind: LocalSourceKind::E0,
            manifest_digest: format!("sha256:{}", "f".repeat(64)),
            snapshot_ref: ArtifactReference {
                tenant_id: "pulso_local".into(),
                id: "018f0f4e-7bbd-7000-8000-000000000699".into(),
                revision: 1,
                digest: format!("sha256:{}", "e".repeat(64)),
            },
            observed_cutoff_rfc3339: "2026-10-03T00:00:00Z".into(),
            execution_mode: "local_simulation".into(),
            simulation_version: "test".into(),
            simulation_seed: "test".into(),
            determinism: "deterministic_given_identical_run_input".into(),
            terminal_status: "complete_simulated".into(),
            formal_route: "do_nothing".into(),
            primary_signal_policy: "local_primary_signal_v3".into(),
            recurrence_measurement_status: if recurrence_unavailable {
                "source_table_unavailable".into()
            } else {
                "observed".into()
            },
            discovery_case_count: 100,
            excluded_replay_case_count: 0,
            signal: primary.clone(),
            signals,
            local_simulation_portfolio: Some(LocalSimulationPortfolio {
                source_family: "e0".into(),
                authority: "simulator_only".into(),
                status: portfolio_status.into(),
                dispositions,
                candidate_signal_digests: candidate_digests,
                primary_signal_digest: primary.as_ref().map(|item| item.digest.clone()),
            }),
            candidates: Vec::new(),
            verification_status: Some("simulated_uncertain".into()),
            proposal: None,
            evaluation: None,
            contact_volume_projection: None,
            snapshot_descriptive_envelope: None,
            events: Vec::new(),
        }
    }

    #[test]
    fn produces_one_descriptive_candidate_for_each_qualified_signal() {
        let technical = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            3,
        );
        let retry = signal(
            "e0_tool_retry_case_rate",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            5,
        );
        let result = result_with(
            vec![technical.clone(), retry.clone()],
            vec![
                LocalSimulationSignalDisposition {
                    metric_id: technical.metric_id.clone(),
                    signal_digest: Some(technical.digest.clone()),
                    state: "candidate_for_simulated_investigation".into(),
                    reason: "measured_threshold_met".into(),
                },
                LocalSimulationSignalDisposition {
                    metric_id: retry.metric_id.clone(),
                    signal_digest: Some(retry.digest.clone()),
                    state: "candidate_for_simulated_investigation".into(),
                    reason: "measured_threshold_met".into(),
                },
            ],
            vec![technical.digest.clone(), retry.digest.clone()],
            "candidates_ready",
        );

        let assembled = assemble_e0_proposals(&result).unwrap();

        assert_eq!(assembled.candidates.len(), 2);
        assert_eq!(
            assembled
                .candidates
                .iter()
                .map(|candidate| candidate.metric_id.as_str())
                .collect::<Vec<_>>(),
            vec!["e0_technical_error_rate", "e0_tool_retry_case_rate"]
        );
        assert!(assembled.candidates.iter().all(|candidate| {
            candidate.source_run_id == result.run_id
                && candidate.source_snapshot_ref.id == result.snapshot_ref.id
                && candidate.source_snapshot_ref.revision == result.snapshot_ref.revision
                && candidate.source_snapshot_ref.digest == result.snapshot_ref.digest
                && candidate.claim_level == "descriptive_only"
                && candidate.route_status == "unlinked"
                && candidate.evaluation_status == "not_evaluated"
                && candidate.business_lift.is_none()
        }));
    }

    #[test]
    fn retains_insufficient_and_nonqualifying_runs_without_inventing_candidates() {
        let mut insufficient = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            0,
        );
        insufficient.missing = 1;
        insufficient.coverage_basis_points = 9_900;
        insufficient.summary_commitment = signal_summary_commitment(&insufficient);
        let insufficient_run = result_with(
            vec![insufficient.clone()],
            vec![LocalSimulationSignalDisposition {
                metric_id: insufficient.metric_id.clone(),
                signal_digest: Some(insufficient.digest.clone()),
                state: "insufficient_evidence".into(),
                reason: "partial_missing_observations".into(),
            }],
            Vec::new(),
            "insufficient_evidence",
        );
        let insufficient_output = assemble_e0_proposals(&insufficient_run).unwrap();
        assert_eq!(
            insufficient_output.status,
            super::LocalProposalAssemblyStatus::InsufficientEvidence
        );
        assert!(insufficient_output.candidates.is_empty());

        let not_qualified = signal(
            "e0_tool_retry_case_rate",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            0,
        );
        let mut no_op_run = result_with(
            vec![not_qualified.clone()],
            vec![LocalSimulationSignalDisposition {
                metric_id: not_qualified.metric_id.clone(),
                signal_digest: Some(not_qualified.digest.clone()),
                state: "not_qualified".into(),
                reason: "no_positive_measured_observation".into(),
            }],
            Vec::new(),
            "no_qualifying_signals",
        );
        no_op_run.terminal_status = "complete_no_opportunity".into();
        let no_op_output = assemble_e0_proposals(&no_op_run).unwrap();
        assert_eq!(
            no_op_output.status,
            super::LocalProposalAssemblyStatus::NoQualifyingSignals
        );
        assert!(no_op_output.candidates.is_empty());
    }

    #[test]
    fn empty_portfolio_is_insufficient_and_original_source_is_unsupported() {
        let mut empty = result_with(Vec::new(), Vec::new(), Vec::new(), "insufficient_evidence");
        empty.signal = None;
        empty
            .local_simulation_portfolio
            .as_mut()
            .unwrap()
            .primary_signal_digest = None;
        let empty_output = assemble_e0_proposals(&empty).unwrap();
        assert_eq!(
            empty_output.status,
            super::LocalProposalAssemblyStatus::InsufficientEvidence
        );
        assert!(empty_output.candidates.is_empty());

        let mut original = empty;
        original.source_kind = LocalSourceKind::OriginalBank;
        original.local_simulation_portfolio = None;
        original.run_id = "private@example.com".into();
        let unsupported = assemble_e0_proposals(&original).unwrap();
        assert_eq!(
            unsupported.status,
            super::LocalProposalAssemblyStatus::Unsupported
        );
        assert!(unsupported.candidates.is_empty());
        assert!(unsupported.business_lift.is_none());
        assert_eq!(unsupported.source_run_id, "unsupported");
    }

    #[test]
    fn candidate_assembly_is_stable_under_signal_and_disposition_permutation() {
        let technical = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            3,
        );
        let retry = signal(
            "e0_tool_retry_case_rate",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            5,
        );
        let dispositions = vec![
            LocalSimulationSignalDisposition {
                metric_id: technical.metric_id.clone(),
                signal_digest: Some(technical.digest.clone()),
                state: "candidate_for_simulated_investigation".into(),
                reason: "measured_threshold_met".into(),
            },
            LocalSimulationSignalDisposition {
                metric_id: retry.metric_id.clone(),
                signal_digest: Some(retry.digest.clone()),
                state: "candidate_for_simulated_investigation".into(),
                reason: "measured_threshold_met".into(),
            },
        ];
        let ordered = result_with(
            vec![technical.clone(), retry.clone()],
            dispositions.clone(),
            vec![technical.digest.clone(), retry.digest.clone()],
            "candidates_ready",
        );
        let mut permuted = result_with(
            vec![retry, technical],
            dispositions.into_iter().rev().collect(),
            vec![
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            ],
            "candidates_ready",
        );
        let selected_primary = ordered.signal.clone();
        permuted.signal = selected_primary.clone();
        permuted
            .local_simulation_portfolio
            .as_mut()
            .unwrap()
            .primary_signal_digest = selected_primary.map(|item| item.digest);

        assert_eq!(
            assemble_e0_proposals(&ordered).unwrap(),
            assemble_e0_proposals(&permuted).unwrap()
        );
    }

    #[test]
    fn rejects_duplicate_metric_rows_and_disposition_digest_mismatch() {
        let technical = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            3,
        );
        let disposition = LocalSimulationSignalDisposition {
            metric_id: technical.metric_id.clone(),
            signal_digest: Some(technical.digest.clone()),
            state: "candidate_for_simulated_investigation".into(),
            reason: "measured_threshold_met".into(),
        };
        let mut duplicate = result_with(
            vec![technical.clone(), technical.clone()],
            vec![disposition.clone()],
            vec![technical.digest.clone()],
            "candidates_ready",
        );
        assert_eq!(
            assemble_e0_proposals(&duplicate),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        duplicate.signals = vec![technical.clone()];
        duplicate
            .local_simulation_portfolio
            .as_mut()
            .unwrap()
            .dispositions[0]
            .signal_digest =
            Some("sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into());
        assert_eq!(
            assemble_e0_proposals(&duplicate),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );
    }

    #[test]
    fn unavailable_recurring_metric_is_explicit_and_has_no_signal_digest() {
        let technical = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            0,
        );
        let result = result_with(
            vec![technical.clone()],
            vec![
                LocalSimulationSignalDisposition {
                    metric_id: technical.metric_id.clone(),
                    signal_digest: Some(technical.digest.clone()),
                    state: "not_qualified".into(),
                    reason: "no_positive_measured_observation".into(),
                },
                LocalSimulationSignalDisposition {
                    metric_id: "e0_recurring_copilot_query_cases".into(),
                    signal_digest: None,
                    state: "unavailable".into(),
                    reason: "source_table_unavailable".into(),
                },
            ],
            Vec::new(),
            "insufficient_evidence",
        );
        let output = assemble_e0_proposals(&result).unwrap();
        let recurring = output
            .dispositions
            .iter()
            .find(|item| item.metric_id == "e0_recurring_copilot_query_cases")
            .unwrap();
        assert_eq!(recurring.state, "unavailable");
        assert_eq!(recurring.signal_digest, None);
        assert_eq!(
            output.status,
            super::LocalProposalAssemblyStatus::InsufficientEvidence
        );

        let mut inconsistent = result;
        inconsistent
            .local_simulation_portfolio
            .as_mut()
            .unwrap()
            .status = "no_qualifying_signals".into();
        assert_eq!(
            assemble_e0_proposals(&inconsistent),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );
    }

    #[test]
    fn rejects_missing_or_unrecognized_disposition_coverage() {
        let technical = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
        );
        let mut missing = result_with(
            vec![technical.clone()],
            vec![LocalSimulationSignalDisposition {
                metric_id: technical.metric_id.clone(),
                signal_digest: Some(technical.digest.clone()),
                state: "candidate_for_simulated_investigation".into(),
                reason: "measured_threshold_met".into(),
            }],
            vec![technical.digest.clone()],
            "candidates_ready",
        );
        missing
            .local_simulation_portfolio
            .as_mut()
            .unwrap()
            .dispositions
            .retain(|item| item.metric_id != "e0_tool_retry_case_rate");
        assert_eq!(
            assemble_e0_proposals(&missing),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        let mut extra = result_with(
            vec![technical.clone()],
            vec![LocalSimulationSignalDisposition {
                metric_id: technical.metric_id.clone(),
                signal_digest: Some(technical.digest.clone()),
                state: "candidate_for_simulated_investigation".into(),
                reason: "measured_threshold_met".into(),
            }],
            vec![technical.digest.clone()],
            "candidates_ready",
        );
        extra
            .local_simulation_portfolio
            .as_mut()
            .unwrap()
            .dispositions
            .push(LocalSimulationSignalDisposition {
                metric_id: "e0_unexpected_metric".into(),
                signal_digest: None,
                state: "unavailable".into(),
                reason: "source_table_unavailable".into(),
            });
        assert_eq!(
            assemble_e0_proposals(&extra),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );
    }

    #[test]
    fn rejects_free_text_reasons_and_invalid_policy_codes_or_versions() {
        let technical = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
        );
        let mut result = result_with(
            vec![technical.clone()],
            vec![LocalSimulationSignalDisposition {
                metric_id: technical.metric_id.clone(),
                signal_digest: Some(technical.digest.clone()),
                state: "candidate_for_simulated_investigation".into(),
                reason: "measured_threshold_met".into(),
            }],
            vec![technical.digest.clone()],
            "candidates_ready",
        );
        result
            .local_simulation_portfolio
            .as_mut()
            .unwrap()
            .dispositions
            .iter_mut()
            .find(|item| item.metric_id == technical.metric_id)
            .unwrap()
            .reason = "customer email: jane@example.com".into();
        assert_eq!(
            assemble_e0_proposals(&result),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        let mut result = result_with(
            vec![technical.clone()],
            vec![LocalSimulationSignalDisposition {
                metric_id: technical.metric_id.clone(),
                signal_digest: Some(technical.digest.clone()),
                state: "candidate_for_simulated_investigation".into(),
                reason: "measured_threshold_met".into(),
            }],
            vec![technical.digest.clone()],
            "candidates_ready",
        );
        result.signals[0].detector_policy_id = "policy with free text".into();
        assert_eq!(
            assemble_e0_proposals(&result),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        result.signals[0].detector_policy_id = "valid_policy_v2".into();
        result.signals[0].detector_policy_version = 0;
        assert_eq!(
            assemble_e0_proposals(&result),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );
    }

    #[test]
    fn rejects_forged_candidate_state_when_metric_does_not_qualify() {
        let technical = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
        );
        let mut result = result_with(
            vec![technical.clone()],
            vec![LocalSimulationSignalDisposition {
                metric_id: technical.metric_id.clone(),
                signal_digest: Some(technical.digest.clone()),
                state: "candidate_for_simulated_investigation".into(),
                reason: "measured_threshold_met".into(),
            }],
            vec![technical.digest.clone()],
            "candidates_ready",
        );
        result.signals[0].numerator = 0;
        assert_eq!(
            assemble_e0_proposals(&result),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        result.signals[0].numerator = 1;
        result.signals[0].denominator = 0;
        assert_eq!(
            assemble_e0_proposals(&result),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );
    }

    #[test]
    fn rejects_measured_field_mutation_when_summary_commitment_is_stale() {
        let technical = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
        );
        let mut result = result_with(
            vec![technical.clone()],
            vec![LocalSimulationSignalDisposition {
                metric_id: technical.metric_id.clone(),
                signal_digest: Some(technical.digest.clone()),
                state: "candidate_for_simulated_investigation".into(),
                reason: "measured_threshold_met".into(),
            }],
            vec![technical.digest.clone()],
            "candidates_ready",
        );
        // It still qualifies and retains its sensor digest/disposition, but its
        // numeric projection no longer matches the producer's commitment.
        result.signals[0].numerator = 2;
        result.signals[0].coverage_basis_points = 9_900;
        assert_eq!(
            assemble_e0_proposals(&result),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );
    }

    #[test]
    fn rejects_inconsistent_coverage_and_overflowing_metric_support() {
        let technical = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
        );
        let mut inconsistent_coverage = result_with(
            vec![technical.clone()],
            vec![LocalSimulationSignalDisposition {
                metric_id: technical.metric_id.clone(),
                signal_digest: Some(technical.digest.clone()),
                state: "candidate_for_simulated_investigation".into(),
                reason: "measured_threshold_met".into(),
            }],
            vec![technical.digest.clone()],
            "candidates_ready",
        );
        inconsistent_coverage.signals[0].coverage_basis_points = 5_000;
        inconsistent_coverage.signals[0].summary_commitment =
            signal_summary_commitment(&inconsistent_coverage.signals[0]);
        assert_eq!(
            assemble_e0_proposals(&inconsistent_coverage),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        let mut overflow = result_with(
            vec![technical.clone()],
            vec![LocalSimulationSignalDisposition {
                metric_id: technical.metric_id.clone(),
                signal_digest: Some(technical.digest.clone()),
                state: "candidate_for_simulated_investigation".into(),
                reason: "measured_threshold_met".into(),
            }],
            vec![technical.digest.clone()],
            "candidates_ready",
        );
        overflow.signals[0].denominator = u64::MAX;
        overflow.signals[0].missing = 1;
        overflow.signals[0].numerator = 1;
        overflow.signals[0].coverage_basis_points = 0;
        overflow.signals[0].summary_commitment = signal_summary_commitment(&overflow.signals[0]);
        assert_eq!(
            assemble_e0_proposals(&overflow),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );
    }

    #[test]
    fn rejects_incomplete_run_envelopes_and_unsafe_provenance_strings() {
        let technical = signal(
            "e0_technical_error_rate",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            1,
        );
        let make_valid = || {
            result_with(
                vec![technical.clone()],
                vec![LocalSimulationSignalDisposition {
                    metric_id: technical.metric_id.clone(),
                    signal_digest: Some(technical.digest.clone()),
                    state: "candidate_for_simulated_investigation".into(),
                    reason: "measured_threshold_met".into(),
                }],
                vec![technical.digest.clone()],
                "candidates_ready",
            )
        };

        let mut invalid = make_valid();
        invalid.execution_mode = "production".into();
        assert_eq!(
            assemble_e0_proposals(&invalid),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        let mut private_tenant = make_valid();
        private_tenant.tenant_id = "jane_doe".into();
        private_tenant.snapshot_ref.tenant_id = "jane_doe".into();
        let safe_output = assemble_e0_proposals(&private_tenant).unwrap();
        let serialized = serde_json::to_string(&safe_output).unwrap();
        assert!(!serialized.contains("jane_doe"));

        let mut invalid = make_valid();
        invalid.terminal_status = "failed".into();
        assert_eq!(
            assemble_e0_proposals(&invalid),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        let mut invalid = make_valid();
        invalid.run_id = "jane@example.com".into();
        assert_eq!(
            assemble_e0_proposals(&invalid),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        let mut invalid = make_valid();
        invalid.run_id = format!("run_1_{}", "1".repeat(100));
        assert_eq!(
            assemble_e0_proposals(&invalid),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        let mut invalid = make_valid();
        invalid.snapshot_ref.id = "customer@example.com".into();
        assert_eq!(
            assemble_e0_proposals(&invalid),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        let mut invalid = make_valid();
        invalid.snapshot_ref.digest = "sha256:not-a-digest".into();
        assert_eq!(
            assemble_e0_proposals(&invalid),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );

        let mut invalid = make_valid();
        invalid.observed_cutoff_rfc3339 = "2026-02-30T00:00:00Z".into();
        assert_eq!(
            assemble_e0_proposals(&invalid),
            Err(super::LocalProposalAssemblyError::InvalidPortfolioEvidence)
        );
    }
}
