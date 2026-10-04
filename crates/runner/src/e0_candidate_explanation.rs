//! Read-only explanation projected from the runner's assembled in-memory E0 result.
//!
//! This is a bounded presentation projection, not another decision engine. It
//! copies only validated lineage, metric aggregates and closed status codes;
//! free-form hypotheses, tenant/customer data, query signatures, prompts and
//! provider/Core payloads are deliberately excluded.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;

const SCHEMA_VERSION: &str = "e0_candidate_explanation_v1";
const RECURRENCE_METRIC: &str = "e0_recurring_copilot_query_cases";
const HOLDOUT_INTERPRETATION: &str = "descriptive_recurrence_only_no_causal_or_outcome_claim";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SnapshotLineage {
    id: String,
    revision: u64,
    digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SourceLineage {
    run_id: String,
    snapshot_ref: SnapshotLineage,
    observed_cutoff_rfc3339: String,
}

struct ReceiptLineage<'a> {
    run_id: &'a str,
    snapshot: &'a SnapshotLineage,
    cutoff: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct MetricEvidence {
    numerator: u64,
    denominator: u64,
    missing: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct HoldoutSummary {
    status: String,
    reproduction_case_count: Option<u64>,
    queried_case_count: Option<u64>,
    matching_case_count: Option<u64>,
    recurrence_rate_basis_points: Option<u16>,
    interpretation: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RouteSummary {
    status: String,
    reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CandidateExplanation {
    proposal_ref: String,
    metric_id: String,
    signal_digest: String,
    summary_commitment: String,
    evidence: Option<MetricEvidence>,
    route: RouteSummary,
    holdout: Option<HoldoutSummary>,
    evaluation_status: &'static str,
    business_lift: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct E0CandidateExplanation {
    schema_version: &'static str,
    projection_kind: &'static str,
    source: SourceLineage,
    assembly_status: String,
    evidence_claim: &'static str,
    candidate_count: usize,
    candidates: Vec<CandidateExplanation>,
    non_candidate_reason: Option<String>,
    dependency_blockers: Vec<String>,
    provider_invoked: bool,
    core_proposal_created: bool,
    core_evaluation_status: &'static str,
    business_lift: Option<String>,
    executable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExplanationError {
    MissingOrInvalidReceipt,
    ReceiptLineageMismatch,
    UnsafeEvidenceState,
    InvalidEventTimeline,
}

/// Build the presentation projection from the assembled in-memory result JSON
/// before it is written. This validates internal consistency, not durable
/// storage, receipt authenticity, or cryptographic digests independently.
/// Only fields in the output structs above can cross this boundary.
pub(crate) fn project_assembled_e0_result(result: &Value) -> Result<Value, ExplanationError> {
    if result.get("source_kind").and_then(Value::as_str) != Some("e0") {
        return Err(ExplanationError::MissingOrInvalidReceipt);
    }

    let assembly = result
        .get("proposal_assembly")
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    let preparation = result
        .get("e0_builder_input_preparation")
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    let run_id = required_code(result, "run_id")?;
    let cutoff = required_code(result, "observed_cutoff_rfc3339")?;
    let assembly_status = required_closed(
        assembly,
        "status",
        &[
            "candidates_ready",
            "insufficient_evidence",
            "no_qualifying_signals",
            "unsupported",
        ],
    )?;
    let unsupported = assembly_status == "unsupported";
    let snapshot = snapshot_lineage(if unsupported {
        result
            .get("snapshot_ref")
            .ok_or(ExplanationError::MissingOrInvalidReceipt)?
    } else {
        assembly
            .get("source_snapshot_ref")
            .ok_or(ExplanationError::MissingOrInvalidReceipt)?
    })?;
    let preparation_snapshot = snapshot_lineage(
        preparation
            .get("source_snapshot_ref")
            .ok_or(ExplanationError::MissingOrInvalidReceipt)?,
    )?;
    let assembly_has_expected_lineage = if unsupported {
        assembled_is_empty(assembly)
            && assembly.get("source_family").and_then(Value::as_str) == Some("e0")
            && assembly.get("authority").and_then(Value::as_str) == Some("none")
            && assembly.get("source_run_id").and_then(Value::as_str) == Some("unsupported")
            && assembly
                .get("source_snapshot_ref")
                .and_then(|item| item.get("id"))
                .and_then(Value::as_str)
                == Some("unsupported")
    } else {
        assembly.get("source_run_id").and_then(Value::as_str) == Some(run_id.as_str())
            && assembly.get("source_family").and_then(Value::as_str) == Some("e0")
            && assembly.get("authority").and_then(Value::as_str) == Some("simulator_only")
            && snapshot_lineage(
                assembly
                    .get("source_snapshot_ref")
                    .ok_or(ExplanationError::MissingOrInvalidReceipt)?,
            )? == snapshot
            && assembly
                .get("observed_cutoff_rfc3339")
                .and_then(Value::as_str)
                == Some(cutoff.as_str())
    };
    if !assembly_has_expected_lineage
        || preparation.get("source_run_id").and_then(Value::as_str) != Some(run_id.as_str())
        || snapshot != preparation_snapshot
        || snapshot_lineage(
            result
                .get("snapshot_ref")
                .ok_or(ExplanationError::MissingOrInvalidReceipt)?,
        )? != snapshot
        || assembly
            .get("observed_cutoff_rfc3339")
            .and_then(Value::as_str)
            != if unsupported {
                Some("")
            } else {
                Some(cutoff.as_str())
            }
        || preparation
            .get("observed_cutoff_rfc3339")
            .and_then(Value::as_str)
            != Some(cutoff.as_str())
    {
        return Err(ExplanationError::ReceiptLineageMismatch);
    }

    let assembled = assembly
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    let prepared = preparation
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    let candidate_count = preparation
        .get("candidate_count")
        .and_then(Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    if assembled.len() != candidate_count
        || prepared.len() != candidate_count
        || (unsupported && candidate_count != 0)
        || (candidate_count == 0 && assembly_status == "candidates_ready")
        || (candidate_count > 0 && assembly_status != "candidates_ready")
    {
        return Err(ExplanationError::ReceiptLineageMismatch);
    }

    let mut blockers = BTreeSet::new();
    let readiness = preparation
        .get("readiness")
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    match preparation.get("status").and_then(Value::as_str) {
        Some("dependency_blocked") if candidate_count > 0 => {
            collect_blocker(readiness, "u20_plan", &mut blockers)?;
            collect_blocker(readiness, "e0_safety_oracle", &mut blockers)?;
        }
        Some("not_applicable") if candidate_count == 0 => {}
        _ => return Err(ExplanationError::UnsafeEvidenceState),
    }
    if preparation.get("provider_invoked") != Some(&Value::Bool(false))
        || preparation.get("executable") != Some(&Value::Bool(false))
        || assembly.get("business_lift") != Some(&Value::Null)
    {
        return Err(ExplanationError::UnsafeEvidenceState);
    }

    let signals = result
        .get("signals")
        .and_then(Value::as_array)
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    let mechanism = result.get("e0_mechanism_resolution");
    let plan = result.get("e0_investigation_proposal_plan");
    let holdout = result.get("e0_recurrence_holdout");
    let mut explanations = Vec::with_capacity(candidate_count);
    let mut seen_candidates = BTreeSet::new();

    for candidate in assembled {
        let metric_id = required_code(candidate, "metric_id")?;
        if ![
            "e0_technical_error_rate",
            "e0_tool_retry_case_rate",
            RECURRENCE_METRIC,
        ]
        .contains(&metric_id.as_str())
        {
            return Err(ExplanationError::UnsafeEvidenceState);
        }
        let proposal_ref = required_code(candidate, "proposal_ref")?;
        let signal_digest = required_digest(candidate, "signal_digest")?;
        let summary_commitment = required_digest(candidate, "summary_commitment")?;
        if candidate.get("source_run_id").and_then(Value::as_str) != Some(run_id.as_str())
            || candidate
                .get("source_snapshot_ref")
                .map(snapshot_lineage)
                .transpose()?
                != Some(snapshot.clone())
            || candidate
                .get("observed_cutoff_rfc3339")
                .and_then(Value::as_str)
                != Some(cutoff.as_str())
            || candidate.get("claim_level").and_then(Value::as_str) != Some("descriptive_only")
            || candidate.get("evaluation_status").and_then(Value::as_str) != Some("not_evaluated")
            || candidate.get("business_lift") != Some(&Value::Null)
            || candidate
                .get("native_agent_core_status")
                .and_then(Value::as_str)
                != Some("not_connected")
            || !seen_candidates.insert((metric_id.clone(), signal_digest.clone()))
        {
            return Err(ExplanationError::UnsafeEvidenceState);
        }
        let matching_preparation = prepared.iter().any(|item| {
            item.get("metric_id").and_then(Value::as_str) == Some(metric_id.as_str())
                && item.get("signal_digest").and_then(Value::as_str) == Some(signal_digest.as_str())
                && item.get("summary_commitment").and_then(Value::as_str)
                    == Some(summary_commitment.as_str())
        });
        if !matching_preparation {
            return Err(ExplanationError::ReceiptLineageMismatch);
        }

        let signal = signals
            .iter()
            .find(|item| {
                item.get("metric_id").and_then(Value::as_str) == Some(metric_id.as_str())
                    && item.get("digest").and_then(Value::as_str) == Some(signal_digest.as_str())
            })
            .ok_or(ExplanationError::ReceiptLineageMismatch)?;
        let evidence = MetricEvidence {
            numerator: required_u64(signal, "numerator")?,
            denominator: required_u64(signal, "denominator")?,
            missing: required_u64(signal, "missing")?,
        };
        if evidence.numerator > evidence.denominator || evidence.missing > evidence.denominator {
            return Err(ExplanationError::UnsafeEvidenceState);
        }

        let route = route_for_candidate(
            candidate,
            mechanism,
            plan,
            &metric_id,
            &signal_digest,
            &ReceiptLineage {
                run_id: &run_id,
                snapshot: &snapshot,
                cutoff: &cutoff,
            },
        )?;
        if route.reason.as_deref() == Some("no_exact_supported_flow_mapping") {
            blockers.insert("no_exact_supported_flow_mapping".to_owned());
        }
        let holdout = if metric_id == RECURRENCE_METRIC {
            let value = holdout.filter(|item| !item.is_null());
            let value = value.ok_or(ExplanationError::MissingOrInvalidReceipt)?;
            validate_holdout_binding(
                value,
                signal.get("pattern_ref").and_then(Value::as_str),
                result.get("manifest_digest").and_then(Value::as_str),
            )?;
            holdout_summary(Some(value))?
        } else {
            None
        };
        explanations.push(CandidateExplanation {
            proposal_ref,
            metric_id,
            signal_digest,
            summary_commitment,
            evidence: Some(evidence),
            route,
            holdout,
            evaluation_status: "not_evaluated",
            business_lift: None,
        });
    }

    validate_timeline(result, candidate_count)?;
    explanations.sort_by(|left, right| left.metric_id.cmp(&right.metric_id));
    let non_candidate_reason = if candidate_count == 0 {
        let reason = required_closed(
            preparation,
            "reason",
            &["no_qualifying_candidate", "proposal_assembly_unsupported"],
        )?;
        if (unsupported && reason != "proposal_assembly_unsupported")
            || (!unsupported && reason != "no_qualifying_candidate")
        {
            return Err(ExplanationError::UnsafeEvidenceState);
        }
        Some(reason)
    } else {
        None
    };
    let projection = E0CandidateExplanation {
        schema_version: SCHEMA_VERSION,
        projection_kind: "assembled_in_memory_result_projection",
        assembly_status,
        source: SourceLineage {
            run_id,
            snapshot_ref: snapshot,
            observed_cutoff_rfc3339: cutoff,
        },
        evidence_claim: "descriptive_only",
        candidate_count,
        candidates: explanations,
        non_candidate_reason,
        dependency_blockers: blockers.into_iter().collect(),
        provider_invoked: false,
        core_proposal_created: false,
        core_evaluation_status: "not_evaluated",
        business_lift: None,
        executable: false,
    };
    serde_json::to_value(projection).map_err(|_| ExplanationError::MissingOrInvalidReceipt)
}

fn route_for_candidate(
    candidate: &Value,
    mechanism: Option<&Value>,
    plan: Option<&Value>,
    metric_id: &str,
    signal_digest: &str,
    lineage: &ReceiptLineage<'_>,
) -> Result<RouteSummary, ExplanationError> {
    let route_status = required_closed(candidate, "route_status", &["unlinked", "mapped"])?;
    let Some(mechanism) = mechanism else {
        if metric_id == RECURRENCE_METRIC || plan.is_some() {
            return Err(ExplanationError::ReceiptLineageMismatch);
        }
        return Ok(RouteSummary {
            status: route_status,
            reason: None,
        });
    };
    let packet = mechanism
        .get("evidence_packet")
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    if packet.get("metric_id").and_then(Value::as_str) != Some(metric_id)
        || packet.get("signal_digest").and_then(Value::as_str) != Some(signal_digest)
        || packet.get("source_run_id").and_then(Value::as_str) != Some(lineage.run_id)
        || packet
            .get("source_snapshot_ref")
            .map(snapshot_lineage)
            .transpose()?
            != Some(lineage.snapshot.clone())
        || packet
            .get("observed_cutoff_rfc3339")
            .and_then(Value::as_str)
            != Some(lineage.cutoff)
        || packet.get("claim_level").and_then(Value::as_str) != Some("descriptive_only")
    {
        return Err(ExplanationError::ReceiptLineageMismatch);
    }
    let resolution = mechanism
        .get("resolution")
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    let status = required_closed(resolution, "status", &["unlinked", "mapped"])?;
    let reason = resolution
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if status != route_status
        || (status == "unlinked" && reason.as_deref() != Some("no_exact_supported_flow_mapping"))
        || (status == "mapped" && reason.is_some())
    {
        return Err(ExplanationError::UnsafeEvidenceState);
    }
    if let Some(plan) = plan {
        if plan.get("proposal_ref").and_then(Value::as_str)
            != candidate.get("proposal_ref").and_then(Value::as_str)
            || plan.get("metric_id").and_then(Value::as_str) != Some(metric_id)
            || plan.get("signal_digest").and_then(Value::as_str) != Some(signal_digest)
            || plan.get("source_run_id").and_then(Value::as_str) != Some(lineage.run_id)
            || plan
                .get("source_snapshot_ref")
                .map(snapshot_lineage)
                .transpose()?
                != Some(lineage.snapshot.clone())
            || plan.get("observed_cutoff_rfc3339").and_then(Value::as_str) != Some(lineage.cutoff)
            || plan.get("claim_level").and_then(Value::as_str) != Some("descriptive_only")
            || plan.get("business_lift") != Some(&Value::Null)
            || plan
                .get("decision")
                .and_then(|decision| decision.get("execution_state"))
                != Some(&Value::String("not_executable".to_owned()))
        {
            return Err(ExplanationError::ReceiptLineageMismatch);
        }
    } else if metric_id == RECURRENCE_METRIC {
        return Err(ExplanationError::MissingOrInvalidReceipt);
    }
    Ok(RouteSummary { status, reason })
}

fn validate_holdout_binding(
    value: &Value,
    expected_candidate_ref: Option<&str>,
    expected_discovery_commitment: Option<&str>,
) -> Result<(), ExplanationError> {
    if expected_candidate_ref.is_none()
        || expected_discovery_commitment.is_none()
        || value.get("candidate_ref").and_then(Value::as_str) != expected_candidate_ref
        || value
            .get("discovery_source_commitment")
            .and_then(Value::as_str)
            != expected_discovery_commitment
    {
        return Err(ExplanationError::ReceiptLineageMismatch);
    }
    Ok(())
}

fn holdout_summary(value: Option<&Value>) -> Result<Option<HoldoutSummary>, ExplanationError> {
    let Some(value) = value.filter(|item| !item.is_null()) else {
        return Ok(None);
    };
    if value.get("interpretation").and_then(Value::as_str) != Some(HOLDOUT_INTERPRETATION) {
        return Err(ExplanationError::UnsafeEvidenceState);
    }
    let status = required_closed(
        value,
        "status",
        &[
            "replicated",
            "not_observed",
            "insufficient_support",
            "unavailable",
        ],
    )?;
    let minimum_support = required_u64(value, "minimum_distinct_case_support")?;
    if minimum_support == 0 {
        return Err(ExplanationError::UnsafeEvidenceState);
    }
    let summary = HoldoutSummary {
        status,
        reproduction_case_count: optional_u64(value, "reproduction_case_count")?,
        queried_case_count: optional_u64(value, "queried_case_count")?,
        matching_case_count: optional_u64(value, "matching_case_count")?,
        recurrence_rate_basis_points: match value.get("recurrence_rate_basis_points") {
            None | Some(Value::Null) => None,
            Some(item) => Some(
                item.as_u64()
                    .and_then(|number| u16::try_from(number).ok())
                    .ok_or(ExplanationError::MissingOrInvalidReceipt)?,
            ),
        },
        interpretation: "descriptive_only",
    };
    let full_counts = summary
        .reproduction_case_count
        .zip(summary.queried_case_count)
        .zip(summary.matching_case_count)
        .zip(summary.recurrence_rate_basis_points)
        .map(|(((reproduced, queried), matching), rate)| (reproduced, queried, matching, rate));
    let consistent_full_counts =
        full_counts.is_some_and(|(reproduced, queried, matching, rate)| {
            queried <= reproduced
                && matching <= queried
                && rate <= 10_000
                && queried >= minimum_support
                && ((u128::from(matching) * 10_000) / u128::from(queried.max(1)))
                    == u128::from(rate)
                && match summary.status.as_str() {
                    "replicated" => matching >= minimum_support,
                    "not_observed" => matching == 0 && rate == 0,
                    _ => false,
                }
        });
    let all_counts_suppressed = summary.reproduction_case_count.is_none()
        && summary.queried_case_count.is_none()
        && summary.matching_case_count.is_none()
        && summary.recurrence_rate_basis_points.is_none();
    if (!matches!(summary.status.as_str(), "replicated" | "not_observed") && !all_counts_suppressed)
        || (matches!(summary.status.as_str(), "replicated" | "not_observed")
            && !consistent_full_counts)
    {
        return Err(ExplanationError::UnsafeEvidenceState);
    }
    Ok(Some(summary))
}

fn collect_blocker(
    readiness: &Value,
    field: &str,
    blockers: &mut BTreeSet<String>,
) -> Result<(), ExplanationError> {
    let state = required_closed(
        readiness,
        field,
        &[
            "unavailable_in_local_simulation",
            "not_requested_without_candidate",
        ],
    )?;
    if state == "unavailable_in_local_simulation" {
        blockers.insert(format!("{field}:{state}"));
    }
    Ok(())
}

fn validate_timeline(result: &Value, candidate_count: usize) -> Result<(), ExplanationError> {
    let events = result
        .get("events")
        .and_then(Value::as_array)
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    let proposal = events
        .iter()
        .find(|event| event.get("stage").and_then(Value::as_str) == Some("proposal_assembly"))
        .ok_or(ExplanationError::InvalidEventTimeline)?;
    let builder = events.iter().find(|event| {
        event.get("stage").and_then(Value::as_str) == Some("e0_builder_input_preparation")
    });
    if proposal.get("detail").and_then(Value::as_str).is_none()
        || (candidate_count > 0
            && builder
                .and_then(|event| event.get("status"))
                .and_then(Value::as_str)
                != Some("dependency_blocked"))
        || (candidate_count == 0
            && builder
                .and_then(|event| event.get("status"))
                .and_then(Value::as_str)
                != Some("not_applicable"))
    {
        return Err(ExplanationError::InvalidEventTimeline);
    }
    Ok(())
}

fn snapshot_lineage(value: &Value) -> Result<SnapshotLineage, ExplanationError> {
    let id = required_code(value, "id")?;
    let revision = value
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    let digest = required_digest(value, "digest")?;
    Ok(SnapshotLineage {
        id,
        revision,
        digest,
    })
}

fn assembled_is_empty(assembly: &Value) -> bool {
    assembly
        .get("candidates")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
}

fn required_code(value: &Value, field: &str) -> Result<String, ExplanationError> {
    let text = value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| {
            !text.is_empty()
                && text.len() <= 160
                && text.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'@')
                })
        })
        .ok_or(ExplanationError::MissingOrInvalidReceipt)?;
    Ok(text.to_owned())
}

fn required_digest(value: &Value, field: &str) -> Result<String, ExplanationError> {
    let digest = required_code(value, field)?;
    let Some(hex) = digest.strip_prefix("sha256:") else {
        return Err(ExplanationError::MissingOrInvalidReceipt);
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ExplanationError::MissingOrInvalidReceipt);
    }
    Ok(digest)
}

fn required_u64(value: &Value, field: &str) -> Result<u64, ExplanationError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or(ExplanationError::MissingOrInvalidReceipt)
}

fn optional_u64(value: &Value, field: &str) -> Result<Option<u64>, ExplanationError> {
    value
        .get(field)
        .filter(|item| !item.is_null())
        .map(|item| {
            item.as_u64()
                .ok_or(ExplanationError::MissingOrInvalidReceipt)
        })
        .transpose()
}

fn required_closed(
    value: &Value,
    field: &str,
    allowed: &[&str],
) -> Result<String, ExplanationError> {
    let state = value
        .get(field)
        .and_then(Value::as_str)
        .filter(|state| allowed.contains(state))
        .ok_or(ExplanationError::UnsafeEvidenceState)?;
    Ok(state.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        ExplanationError, holdout_summary, project_assembled_e0_result, validate_holdout_binding,
    };
    use serde_json::{Value, json};

    fn persisted_candidate_receipts() -> Value {
        json!({
            "source_kind": "e0",
            "run_id": "run_01",
            "observed_cutoff_rfc3339": "2025-07-01T00:00:00Z",
            "snapshot_ref": {
                "tenant_id": "tenant_internal",
                "id": "snapshot_01", "revision": 1,
                "digest": format!("sha256:{}", "b".repeat(64))
            },
            "signals": [{
                "metric_id": "e0_technical_error_rate",
                "digest": format!("sha256:{}", "a".repeat(64)),
                "numerator": 2,
                "denominator": 10,
                "missing": 1
            }],
            "proposal_assembly": {
                "status": "candidates_ready",
                "source_family": "e0",
                "authority": "simulator_only",
                "source_run_id": "run_01",
                "source_snapshot_ref": {
                    "id": "snapshot_01", "revision": 1,
                    "digest": format!("sha256:{}", "b".repeat(64))
                },
                "observed_cutoff_rfc3339": "2025-07-01T00:00:00Z",
                "business_lift": null,
                "candidates": [{
                    "proposal_ref": "run_01:sha256:signal",
                    "source_run_id": "run_01",
                    "source_snapshot_ref": {
                        "id": "snapshot_01", "revision": 1,
                        "digest": format!("sha256:{}", "b".repeat(64))
                    },
                    "observed_cutoff_rfc3339": "2025-07-01T00:00:00Z",
                    "metric_id": "e0_technical_error_rate",
                    "signal_digest": format!("sha256:{}", "a".repeat(64)),
                    "summary_commitment": format!("sha256:{}", "c".repeat(64)),
                    "hypothesis": "raw private hypothesis must not pass through",
                    "claim_level": "descriptive_only",
                    "route_status": "unlinked",
                    "evaluation_status": "not_evaluated",
                    "business_lift": null,
                    "native_agent_core_status": "not_connected"
                }]
            },
            "e0_builder_input_preparation": {
                "source_run_id": "run_01",
                "source_snapshot_ref": {
                    "id": "snapshot_01", "revision": 1,
                    "digest": format!("sha256:{}", "b".repeat(64))
                },
                "observed_cutoff_rfc3339": "2025-07-01T00:00:00Z",
                "status": "dependency_blocked",
                "candidate_count": 1,
                "candidates": [{
                    "metric_id": "e0_technical_error_rate",
                    "signal_digest": format!("sha256:{}", "a".repeat(64)),
                    "summary_commitment": format!("sha256:{}", "c".repeat(64))
                }],
                "readiness": {
                    "u20_plan": "unavailable_in_local_simulation",
                    "e0_safety_oracle": "unavailable_in_local_simulation"
                },
                "provider_invoked": false,
                "executable": false
            },
            "e0_recurrence_holdout": null,
            "events": [
                {"stage":"proposal_assembly","status":"candidates_ready","detail":"candidate_count=1"},
                {"stage":"e0_builder_input_preparation","status":"dependency_blocked"}
            ]
        })
    }

    #[test]
    fn explanation_is_deterministic_and_projects_only_safe_receipt_fields() {
        let receipts = persisted_candidate_receipts();
        let first = project_assembled_e0_result(&receipts).expect("valid assembled result");
        let second = project_assembled_e0_result(&receipts).expect("same assembled result");
        assert_eq!(first, second);
        assert_eq!(first["candidate_count"], 1);
        assert_eq!(
            first["projection_kind"],
            "assembled_in_memory_result_projection"
        );
        assert_eq!(first["candidates"][0]["evidence"]["numerator"], 2);
        assert_eq!(first["candidates"][0]["evidence"]["denominator"], 10);
        assert_eq!(first["dependency_blockers"].as_array().unwrap().len(), 2);
        assert!(!first.to_string().contains("raw private hypothesis"));
        assert_eq!(first["business_lift"], Value::Null);
        assert_eq!(first["core_proposal_created"], false);
        assert_eq!(first["executable"], false);
    }

    #[test]
    fn explanation_rejects_cross_run_or_false_ready_receipts() {
        let mut cross_run = persisted_candidate_receipts();
        cross_run["e0_builder_input_preparation"]["source_run_id"] = json!("run_other");
        assert_eq!(
            project_assembled_e0_result(&cross_run),
            Err(ExplanationError::ReceiptLineageMismatch)
        );

        let mut false_ready = persisted_candidate_receipts();
        false_ready["e0_builder_input_preparation"]["provider_invoked"] = json!(true);
        assert_eq!(
            project_assembled_e0_result(&false_ready),
            Err(ExplanationError::UnsafeEvidenceState)
        );
    }

    #[test]
    fn holdout_requires_selected_candidate_and_discovery_source_binding() {
        let holdout = json!({
            "candidate_ref": "sha256:selected-pattern",
            "discovery_source_commitment": "sha256:discovery",
            "status": "replicated",
            "reproduction_case_count": 12,
            "queried_case_count": 10,
            "matching_case_count": 4,
            "recurrence_rate_basis_points": 4000,
            "interpretation": "descriptive_recurrence_only_no_causal_or_outcome_claim"
        });
        assert_eq!(
            validate_holdout_binding(
                &holdout,
                Some("sha256:selected-pattern"),
                Some("sha256:discovery")
            ),
            Ok(())
        );
        assert_eq!(
            validate_holdout_binding(
                &holdout,
                Some("sha256:other-pattern"),
                Some("sha256:discovery")
            ),
            Err(ExplanationError::ReceiptLineageMismatch)
        );
        assert_eq!(
            validate_holdout_binding(&holdout, None, Some("sha256:discovery")),
            Err(ExplanationError::ReceiptLineageMismatch)
        );
        let mut missing_candidate = holdout.clone();
        missing_candidate
            .as_object_mut()
            .unwrap()
            .remove("candidate_ref");
        assert_eq!(
            validate_holdout_binding(
                &missing_candidate,
                Some("sha256:selected-pattern"),
                Some("sha256:discovery")
            ),
            Err(ExplanationError::ReceiptLineageMismatch)
        );
        assert_eq!(
            validate_holdout_binding(
                &holdout,
                Some("sha256:selected-pattern"),
                Some("sha256:other-source")
            ),
            Err(ExplanationError::ReceiptLineageMismatch)
        );
    }

    #[test]
    fn holdout_statuses_require_complete_consistent_counts() {
        let replicated = json!({
            "minimum_distinct_case_support": 3,
            "status": "replicated", "reproduction_case_count": 12,
            "queried_case_count": 10, "matching_case_count": 4,
            "recurrence_rate_basis_points": 4000,
            "interpretation": "descriptive_recurrence_only_no_causal_or_outcome_claim"
        });
        assert!(holdout_summary(Some(&replicated)).unwrap().is_some());

        let mut inconsistent = replicated.clone();
        inconsistent["matching_case_count"] = json!(0);
        assert_eq!(
            holdout_summary(Some(&inconsistent)),
            Err(ExplanationError::UnsafeEvidenceState)
        );
        let not_observed = json!({
            "minimum_distinct_case_support": 3,
            "status": "not_observed", "reproduction_case_count": 12,
            "queried_case_count": 10, "matching_case_count": 0,
            "recurrence_rate_basis_points": 1,
            "interpretation": "descriptive_recurrence_only_no_causal_or_outcome_claim"
        });
        assert_eq!(
            holdout_summary(Some(&not_observed)),
            Err(ExplanationError::UnsafeEvidenceState)
        );
        let malformed_unavailable = json!({
            "minimum_distinct_case_support": 3,
            "status": "unavailable", "reproduction_case_count": 12,
            "queried_case_count": null, "matching_case_count": null,
            "recurrence_rate_basis_points": null,
            "interpretation": "descriptive_recurrence_only_no_causal_or_outcome_claim"
        });
        assert_eq!(
            holdout_summary(Some(&malformed_unavailable)),
            Err(ExplanationError::UnsafeEvidenceState)
        );
        let unavailable = json!({
            "minimum_distinct_case_support": 3,
            "status": "unavailable", "reproduction_case_count": null,
            "queried_case_count": null, "matching_case_count": null,
            "recurrence_rate_basis_points": null,
            "interpretation": "descriptive_recurrence_only_no_causal_or_outcome_claim"
        });
        assert!(holdout_summary(Some(&unavailable)).unwrap().is_some());
        let insufficient = json!({
            "minimum_distinct_case_support": 3,
            "status": "insufficient_support", "reproduction_case_count": null,
            "queried_case_count": null, "matching_case_count": null,
            "recurrence_rate_basis_points": null,
            "interpretation": "descriptive_recurrence_only_no_causal_or_outcome_claim"
        });
        assert!(holdout_summary(Some(&insufficient)).unwrap().is_some());
    }
}
