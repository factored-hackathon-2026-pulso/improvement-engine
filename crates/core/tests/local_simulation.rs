use improvement_engine_core::ArtifactReference;
use improvement_engine_core::local_simulation::{
    LocalObservedEvent, LocalObservedQuery, LocalRunError, LocalRunInput, LocalRunMetadata,
    LocalSnapshotComplaintAggregate, LocalSnapshotComplaintProjection,
    LocalSnapshotContactAggregate, LocalSnapshotContactProjection, LocalSourceKind,
    run_local_simulation,
};

fn snapshot() -> ArtifactReference {
    ArtifactReference {
        tenant_id: "pulso_local".into(),
        id: "0199b21c-7eab-7000-8000-000000000001".into(),
        revision: 1,
        digest: format!("sha256:{}", "a".repeat(64)),
    }
}

fn complaint_projection(
    status: &str,
    missing_fields: Vec<String>,
) -> LocalSnapshotComplaintProjection {
    let aggregates = if status == "supported" {
        vec![LocalSnapshotComplaintAggregate::new(
            "2026-08",
            "transactional",
            "phone",
            5,
            false,
            Some(5),
            Some(0),
            Some(5),
            false,
            Some(5),
            Some(0),
            Some(4.0),
            false,
            Some(5),
            Some(0),
            Some(2.0),
        )]
    } else {
        Vec::new()
    };
    LocalSnapshotComplaintProjection::new(
        status,
        missing_fields,
        "literal_source_wall_clock_month",
        "final_extract_facts_only",
        "partial",
        1,
        5,
        if status == "supported" { 5 } else { 0 },
        aggregates,
    )
    .unwrap()
}

#[test]
fn original_snapshot_emits_descriptive_non_publishable_improvement_envelope() {
    let projection = LocalSnapshotContactProjection::new(
        1,
        5,
        10,
        vec![
            LocalSnapshotContactAggregate::new("2026-08", "complaint", "phone", 5),
            LocalSnapshotContactAggregate::new("2026-09", "complaint", "phone", 5),
        ],
    )
    .unwrap();
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "original-snapshot-run",
            "pulso_local",
            LocalSourceKind::OriginalBank,
            format!("sha256:{}", "b".repeat(64)),
            snapshot(),
            1_751_328_000,
            "2025-07-01T00:00:00Z",
        ),
        Vec::new(),
        0,
        Vec::new(),
    )
    .with_snapshot_descriptive_contact_projection(projection);

    let result = run_local_simulation(input).unwrap();
    assert_eq!(
        result.complaint_projection_status.as_ref().unwrap().status,
        "absent"
    );
    let envelope = result.snapshot_descriptive_envelope.unwrap();
    assert_eq!(
        envelope.finding.temporal_basis,
        "literal_source_wall_clock_month"
    );
    assert_eq!(envelope.finding.value_semantics, "final_extract_facts_only");
    assert_eq!(envelope.finding.coverage, "partial");
    assert_eq!(envelope.finding.literal_months, ["2026-08", "2026-09"]);
    assert_eq!(envelope.finding.supported_contact_count, 10);
    assert_eq!(envelope.finding.minimum_cell_count, 5);
    assert_eq!(
        envelope.finding.source_snapshot_digest,
        format!("sha256:{}", "a".repeat(64))
    );
    assert_eq!(
        envelope.agent_core_candidate,
        "dependency_blocked_snapshot_semantics"
    );
    assert_eq!(envelope.proposal.status, "simulated_unverified");
    assert_eq!(envelope.proposal.execution_status, "not_executed");
    assert!(!envelope.proposal.publication_eligible);
    assert_eq!(result.formal_route, "do_nothing");

    let json = serde_json::to_value(envelope).unwrap();
    let serialized = serde_json::to_string(&json).unwrap();
    assert!(json["finding"].get("rejected_rows").is_none());
    assert!(json["finding"].get("suppressed_cells").is_none());
    for forbidden in ["observed_cutoff", "customer_id", "interaction_id"] {
        assert!(!serialized.to_ascii_lowercase().contains(forbidden));
    }
    assert_eq!(
        json["finding"]["claim_scope"],
        "descriptive_only_no_causal_or_roi_claim"
    );
}

#[test]
fn original_snapshot_can_emit_pqr_only_evidence_without_fabricating_contact_counts() {
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "original-pqr-only-run",
            "pulso_local",
            LocalSourceKind::OriginalBank,
            format!("sha256:{}", "b".repeat(64)),
            snapshot(),
            1_751_328_000,
            "2025-07-01T00:00:00Z",
        ),
        Vec::new(),
        0,
        Vec::new(),
    )
    .with_snapshot_descriptive_complaint_projection(complaint_projection("supported", Vec::new()));

    let result = run_local_simulation(input).unwrap();
    let envelope = result.snapshot_descriptive_envelope.unwrap();
    assert_eq!(
        envelope.finding.signal_id,
        "original_complaints_literal_month_descriptive_v1"
    );
    assert_eq!(envelope.finding.supported_contact_count, 0);
    assert_eq!(envelope.finding.complaint_contact_count, 0);
    let pqr = envelope
        .finding
        .complaint_table_projection
        .as_ref()
        .unwrap();
    assert_eq!(pqr.policy_id(), "original_complaint_literal_month_k_v1");
    assert_eq!(pqr.included_complaint_count(), 5);
    assert_eq!(pqr.aggregates()[0].complaint_count, 5);
    assert_eq!(
        result.complaint_projection_status.unwrap().status,
        "supported"
    );
    assert_eq!(result.formal_route, "do_nothing");
    assert!(!envelope.proposal.publication_eligible);
    let serialized = serde_json::to_string(&envelope).unwrap();
    assert!(serialized.contains("final_extract_facts_only"));
    assert!(serialized.contains("descriptive_only_no_causal_or_roi_claim"));
    for forbidden in [
        "customer_id",
        "complaint_id",
        "description",
        "observed_cutoff",
    ] {
        assert!(!serialized.contains(forbidden));
    }
}

#[test]
fn unsupported_complaint_projection_is_distinct_from_absent_and_emits_no_finding() {
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "original-pqr-unsupported-run",
            "pulso_local",
            LocalSourceKind::OriginalBank,
            format!("sha256:{}", "b".repeat(64)),
            snapshot(),
            1_751_328_000,
            "2025-07-01T00:00:00Z",
        ),
        Vec::new(),
        0,
        Vec::new(),
    )
    .with_snapshot_descriptive_complaint_projection(complaint_projection(
        "unsupported",
        vec!["usable_grouping_rows".into()],
    ));

    let result = run_local_simulation(input).unwrap();
    assert_eq!(
        result.complaint_projection_status.as_ref().unwrap().status,
        "unsupported"
    );
    assert_eq!(
        result
            .complaint_projection_status
            .as_ref()
            .unwrap()
            .missing_fields,
        ["usable_grouping_rows"]
    );
    assert!(result.snapshot_descriptive_envelope.is_none());
    assert_eq!(result.terminal_status, "unsupported_source");
}

#[test]
fn unsupported_complaint_projection_rejects_unbounded_diagnostic_codes() {
    let projection = LocalSnapshotComplaintProjection::new(
        "unsupported",
        vec!["alex".into()],
        "literal_source_wall_clock_month",
        "final_extract_facts_only",
        "partial",
        1,
        5,
        0,
        Vec::new(),
    );
    assert!(projection.is_err());

    let repeated_diagnostics = LocalSnapshotComplaintProjection::new(
        "unsupported",
        vec!["usable_grouping_rows".into(); 6],
        "literal_source_wall_clock_month",
        "final_extract_facts_only",
        "partial",
        1,
        5,
        0,
        Vec::new(),
    );
    assert!(repeated_diagnostics.is_err());

    let duplicate_diagnostics = LocalSnapshotComplaintProjection::new(
        "unsupported",
        vec!["usable_grouping_rows".into(), "usable_grouping_rows".into()],
        "literal_source_wall_clock_month",
        "final_extract_facts_only",
        "partial",
        1,
        5,
        0,
        Vec::new(),
    );
    assert!(duplicate_diagnostics.is_err());
}

#[test]
fn usable_complaint_source_with_only_suppressed_cells_reports_no_public_evidence() {
    let projection = LocalSnapshotComplaintProjection::new(
        "supported",
        Vec::new(),
        "literal_source_wall_clock_month",
        "final_extract_facts_only",
        "partial",
        1,
        5,
        0,
        Vec::new(),
    )
    .unwrap();
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "original-pqr-no-visible-cells-run",
            "pulso_local",
            LocalSourceKind::OriginalBank,
            format!("sha256:{}", "b".repeat(64)),
            snapshot(),
            1_751_328_000,
            "2025-07-01T00:00:00Z",
        ),
        Vec::new(),
        0,
        Vec::new(),
    )
    .with_snapshot_descriptive_complaint_projection(projection);

    let result = run_local_simulation(input).unwrap();
    let status = result.complaint_projection_status.unwrap();
    assert_eq!(status.status, "supported_no_reportable_cells");
    assert!(result.snapshot_descriptive_envelope.is_none());
    assert!(result.events.iter().any(|event| {
        event.stage == "complaint_projection"
            && event.status == "supported_no_reportable_cells"
            && event.detail.contains("no complaint evidence admitted")
    }));
    assert!(!result.events.iter().any(|event| {
        event.stage == "detection" && event.status == "snapshot_projection_complete"
    }));
}

#[test]
fn complaint_projection_rejects_undersuppressed_duplicate_and_inconsistent_cells() {
    let cell = || {
        LocalSnapshotComplaintAggregate::new(
            "2026-08",
            "transactional",
            "phone",
            5,
            false,
            Some(5),
            Some(0),
            Some(5),
            false,
            Some(5),
            Some(0),
            Some(4.0),
            false,
            Some(5),
            Some(0),
            Some(2.0),
        )
    };
    let build = |minimum, count, cells| {
        LocalSnapshotComplaintProjection::new(
            "supported",
            Vec::new(),
            "literal_source_wall_clock_month",
            "final_extract_facts_only",
            "partial",
            1,
            minimum,
            count,
            cells,
        )
    };
    assert!(build(4, 5, vec![cell()]).is_err());
    assert!(build(5, 10, vec![cell(), cell()]).is_err());
    let inconsistent = LocalSnapshotComplaintAggregate::new(
        "2026-08",
        "transactional",
        "phone",
        5,
        false,
        Some(4),
        Some(1),
        Some(2),
        false,
        Some(5),
        Some(0),
        Some(4.0),
        false,
        Some(5),
        Some(0),
        Some(2.0),
    );
    assert!(build(5, 5, vec![inconsistent]).is_err());
}

#[test]
fn complaint_projection_rejects_small_metric_denominators_unless_suppressed() {
    let build = |cell| {
        LocalSnapshotComplaintProjection::new(
            "supported",
            Vec::new(),
            "literal_source_wall_clock_month",
            "final_extract_facts_only",
            "partial",
            1,
            5,
            5,
            vec![cell],
        )
    };
    let small_binary_group = LocalSnapshotComplaintAggregate::new(
        "2026-08",
        "transactional",
        "phone",
        5,
        false,
        Some(5),
        Some(0),
        Some(1),
        false,
        Some(5),
        Some(0),
        Some(4.0),
        false,
        Some(5),
        Some(0),
        Some(2.0),
    );
    assert!(build(small_binary_group).is_err());

    let small_numeric_denominator = LocalSnapshotComplaintAggregate::new(
        "2026-08",
        "transactional",
        "phone",
        5,
        false,
        Some(5),
        Some(0),
        Some(5),
        true,
        None,
        None,
        None,
        false,
        Some(5),
        Some(0),
        Some(2.0),
    );
    assert!(build(small_numeric_denominator).is_ok());
}

#[test]
fn complaint_projection_cannot_be_attached_to_e0_source() {
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "e0-wrong-complaint-projection",
            "pulso_local",
            LocalSourceKind::E0,
            format!("sha256:{}", "b".repeat(64)),
            snapshot(),
            1_751_328_000,
            "2025-07-01T00:00:00Z",
        ),
        vec![1],
        0,
        vec![],
    )
    .with_snapshot_descriptive_complaint_projection(complaint_projection("supported", Vec::new()));

    assert!(matches!(
        run_local_simulation(input),
        Err(LocalRunError::InvalidEventProjection)
    ));
}

#[test]
fn snapshot_projection_rejects_unsafe_k_invalid_month_and_duplicate_cells() {
    let cell =
        |month, count| LocalSnapshotContactAggregate::new(month, "complaint", "phone", count);
    assert!(LocalSnapshotContactProjection::new(1, 4, 5, vec![cell("2026-08", 5)]).is_err());
    assert!(LocalSnapshotContactProjection::new(1, 5, 5, vec![cell("2026-13", 5)]).is_err());
    assert!(
        LocalSnapshotContactProjection::new(
            1,
            5,
            10,
            vec![cell("2026-08", 5), cell("2026-08", 5)],
        )
        .is_err()
    );
}

#[test]
fn snapshot_projection_cannot_be_attached_to_e0_source() {
    let projection = LocalSnapshotContactProjection::new(
        1,
        5,
        5,
        vec![LocalSnapshotContactAggregate::new(
            "2026-08",
            "complaint",
            "phone",
            5,
        )],
    )
    .unwrap();
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "e0-wrong-source",
            "pulso_local",
            LocalSourceKind::E0,
            format!("sha256:{}", "b".repeat(64)),
            snapshot(),
            1_751_328_000,
            "2025-07-01T00:00:00Z",
        ),
        vec![1],
        0,
        vec![],
    )
    .with_snapshot_descriptive_contact_projection(projection);

    assert!(matches!(
        run_local_simulation(input),
        Err(LocalRunError::InvalidEventProjection)
    ));
}

fn event(
    case_ordinal: u32,
    event_ordinal: u32,
    technical_error: Option<bool>,
    route_code: Option<&str>,
    actor_layer: Option<&str>,
    tool_code: Option<&str>,
) -> LocalObservedEvent {
    LocalObservedEvent {
        case_ordinal,
        event_ordinal,
        parent_event_ordinal: None,
        event_time: format!("2026-08-01T00:00:{:02}Z", event_ordinal),
        event_kind: "tool_call".into(),
        route_code: route_code.map(str::to_owned),
        actor_layer: actor_layer.map(str::to_owned),
        tool_code: tool_code.map(str::to_owned),
        technical_error,
        retry_count: None,
        approval: None,
        signal_code: None,
    }
}

#[test]
fn retry_metric_counts_cases_with_known_counts_and_keeps_missing_explicit() {
    let mut retried = event(
        1,
        1,
        Some(false),
        Some("payments"),
        Some("tree"),
        Some("status_lookup"),
    );
    retried.retry_count = Some(2);
    let mut additional_zero_retry_call = event(
        1,
        2,
        Some(false),
        Some("payments"),
        Some("tree"),
        Some("status_lookup"),
    );
    additional_zero_retry_call.retry_count = Some(0);
    let mut known_zero = event(
        2,
        1,
        Some(false),
        Some("payments"),
        Some("tree"),
        Some("status_lookup"),
    );
    known_zero.retry_count = Some(0);
    let unknown = event(
        3,
        1,
        Some(false),
        Some("payments"),
        Some("tree"),
        Some("status_lookup"),
    );
    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-known-missing",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:abababababababababababababababababababababababababababababababab",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        vec![1, 2, 3, 4],
        0,
        vec![retried, additional_zero_retry_call, known_zero, unknown],
    ))
    .expect("retry observations are measured");

    let detection_detail = &result
        .events
        .iter()
        .find(|event| event.stage == "detection")
        .expect("detection timeline event")
        .detail;
    assert!(detection_detail.contains("retries"));
    assert!(!detection_detail.contains("technical errors"));

    let retry = result
        .signals
        .iter()
        .find(|signal| signal.metric_id == "e0_tool_retry_case_rate")
        .expect("retry metric is visible");
    assert_eq!(
        (retry.numerator, retry.denominator, retry.missing),
        (1, 2, 2)
    );
    assert_eq!(retry.minimum_support, 1);
    let proposal = result
        .proposal
        .expect("positive retry signal yields a review draft");
    assert_eq!(proposal.evidence.metric_id, "e0_tool_retry_case_rate");
    assert!(proposal.hypothesis.contains("retry"));
    assert!(
        proposal
            .hypothesis
            .contains("does not establish cause or savings")
    );
    assert_eq!(
        proposal.proposed_artifact["observed_evidence"]["retry_cases"],
        1
    );
    assert_eq!(
        proposal.proposed_artifact["observed_evidence"]["retry_denominator_known_cases"],
        2
    );
    assert_eq!(
        proposal.proposed_artifact["observed_evidence"]["retry_cases_missing"],
        2
    );
}

#[test]
fn simulated_proposal_reports_privacy_gated_retry_error_cooccurrence() {
    let mut events = Vec::new();
    for case_ordinal in 1..=5 {
        let mut observed = event(case_ordinal, 1, Some(true), None, None, None);
        observed.retry_count = Some(1);
        events.push(observed);
    }
    for case_ordinal in 6..=10 {
        let mut observed = event(case_ordinal, 1, Some(false), None, None, None);
        observed.retry_count = Some(2);
        events.push(observed);
    }
    let mut unknown_error = event(11, 1, None, None, None, None);
    unknown_error.retry_count = Some(1);
    events.push(unknown_error);
    let mut known_no_retry_with_error = event(12, 1, Some(true), None, None, None);
    known_no_retry_with_error.retry_count = Some(0);
    events.push(known_no_retry_with_error);
    let mut known_no_retry = event(13, 1, Some(false), None, None, None);
    known_no_retry.retry_count = Some(0);
    events.push(known_no_retry);

    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-error-overlap",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:abababababababababababababababababababababababababababababababab",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=13).collect(),
        0,
        events,
    ))
    .expect("retry/error case overlap is summarized");

    let proposal = result
        .proposal
        .expect("qualifying evidence produces a draft");
    let overlap = &proposal.proposed_artifact["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["status"], "reportable");
    assert_eq!(overlap["policy_id"], "e0_retry_error_overlap_k_v2");
    assert_eq!(overlap["policy_version"], 2);
    assert_eq!(overlap["minimum_reportable_cases"], 5);
    assert_eq!(overlap["retry_positive_cases_with_known_error_status"], 10);
    assert_eq!(overlap["retry_and_technical_error_cases"], 5);
    assert_eq!(
        overlap["error_rate_within_retry_positive_known_error_status_basis_points"],
        5000
    );
    assert_eq!(
        overlap["unknown_error_status_policy"],
        "retry-positive cases without explicit technical-error status are excluded; their count is not serialized"
    );
    assert!(
        overlap["interpretation"]
            .as_str()
            .unwrap()
            .contains("co-occurrence")
    );
    assert!(
        overlap["interpretation"]
            .as_str()
            .unwrap()
            .contains("not causal")
    );
}

#[test]
fn simulated_proposal_suppresses_small_retry_error_overlap_cells() {
    let events = (1..=8)
        .map(|case_ordinal| {
            let technical_error = case_ordinal <= 5;
            let mut observed = event(case_ordinal, 1, Some(technical_error), None, None, None);
            observed.retry_count = Some(1);
            observed
        })
        .collect();
    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-error-overlap-small-cells",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:acacacacacacacacacacacacacacacacacacacacacacacacacacacacacacacac",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=8).collect(),
        0,
        events,
    ))
    .expect("small overlap does not block the simulated proposal");

    let proposal = result.proposal.expect("technical-error signal qualifies");
    let overlap = &proposal.proposed_artifact["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["status"], "suppressed_below_minimum_support");
    assert_eq!(overlap["policy_id"], "e0_retry_error_overlap_k_v2");
    assert_eq!(
        overlap["retry_positive_cases_with_known_error_status"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["retry_and_technical_error_cases"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["error_rate_within_retry_positive_known_error_status_basis_points"],
        serde_json::Value::Null
    );
    assert!(
        overlap["interpretation"]
            .as_str()
            .unwrap()
            .contains("withheld")
    );
}

#[test]
fn retry_error_overlap_hides_retry_support_when_error_status_is_missing() {
    let mut events = Vec::new();
    for case_ordinal in 1..=6 {
        let mut observed = event(case_ordinal, 1, None, None, None, None);
        observed.retry_count = Some(1);
        events.push(observed);
    }
    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-error-overlap-missing-error-status",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:adadadadadadadadadadadadadadadadadadadadadadadadadadadadadadadad",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=6).collect(),
        0,
        events,
    ))
    .expect("unknown error status does not become a no-retry observation");

    let proposal = result.proposal.expect("positive retries qualify for draft");
    let overlap = &proposal.proposed_artifact["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["status"], "suppressed_below_minimum_support");
    assert_eq!(
        overlap["retry_positive_cases_with_known_error_status"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["retry_and_technical_error_cases"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["error_rate_within_retry_positive_known_error_status_basis_points"],
        serde_json::Value::Null
    );
    assert!(
        overlap["interpretation"]
            .as_str()
            .unwrap()
            .contains("support")
    );
    assert!(
        !overlap["interpretation"]
            .as_str()
            .unwrap()
            .contains("retry-positive")
    );
}

#[test]
fn retry_error_overlap_coarsens_sub_k_positive_with_missing_error_status() {
    let mut events = Vec::new();
    let mut positive_missing_error = event(1, 1, None, None, None, None);
    positive_missing_error.retry_count = Some(1);
    events.push(positive_missing_error);
    for case_ordinal in 2..=6 {
        let mut known_zero_with_error = event(case_ordinal, 1, Some(true), None, None, None);
        known_zero_with_error.retry_count = Some(0);
        events.push(known_zero_with_error);
    }
    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-error-overlap-sub-k-positive-missing-error",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4b4",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=6).collect(),
        0,
        events,
    ))
    .expect("a small known retry with unknown error status must remain private");

    let proposal = result
        .proposal
        .expect("known technical errors qualify for draft");
    let overlap = &proposal.proposed_artifact["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["status"], "suppressed_below_minimum_support");
    assert_eq!(
        overlap["retry_positive_cases_with_known_error_status"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["retry_and_technical_error_cases"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["error_rate_within_retry_positive_known_error_status_basis_points"],
        serde_json::Value::Null
    );
    assert!(
        !overlap["interpretation"]
            .as_str()
            .unwrap()
            .contains("retry-positive")
    );
}

#[test]
fn retry_error_overlap_does_not_call_all_missing_retry_counts_no_retries() {
    let events = (1..=6)
        .map(|case_ordinal| event(case_ordinal, 1, Some(true), None, None, None))
        .collect();
    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-error-overlap-all-retry-status-missing",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:aeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeaeae",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=6).collect(),
        0,
        events,
    ))
    .expect("missing retry counts do not block the simulated proposal");

    let proposal = result.proposal.expect("technical errors qualify for draft");
    let overlap = &proposal.proposed_artifact["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["status"], "insufficient_retry_status_coverage");
    assert_eq!(
        overlap["retry_positive_cases_with_known_error_status"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["retry_and_technical_error_cases"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["error_rate_within_retry_positive_known_error_status_basis_points"],
        serde_json::Value::Null
    );
    assert!(
        overlap["interpretation"]
            .as_str()
            .unwrap()
            .contains("retry status coverage")
    );
}

#[test]
fn retry_error_overlap_does_not_call_partial_retry_coverage_no_retries() {
    let mut events = (1..=6)
        .map(|case_ordinal| {
            let mut observed = event(case_ordinal, 1, Some(true), None, None, None);
            observed.retry_count = Some(0);
            observed
        })
        .collect::<Vec<_>>();
    events.push(event(7, 1, Some(true), None, None, None));
    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-error-overlap-partial-retry-status-missing",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:afafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafaf",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=7).collect(),
        0,
        events,
    ))
    .expect("partial retry status does not block the simulated proposal");

    let proposal = result.proposal.expect("technical errors qualify for draft");
    let overlap = &proposal.proposed_artifact["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["status"], "insufficient_retry_status_coverage");
    assert_eq!(
        overlap["retry_positive_cases_with_known_error_status"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["retry_and_technical_error_cases"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["error_rate_within_retry_positive_known_error_status_basis_points"],
        serde_json::Value::Null
    );
    assert!(
        overlap["interpretation"]
            .as_str()
            .unwrap()
            .contains("retry status coverage")
    );
}

#[test]
fn retry_error_overlap_does_not_disclose_zero_retry_support_categorically() {
    let events = (1..=6)
        .map(|case_ordinal| {
            let mut observed = event(case_ordinal, 1, Some(true), None, None, None);
            observed.retry_count = Some(0);
            observed
        })
        .collect();
    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-error-overlap-known-no-retries",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=6).collect(),
        0,
        events,
    ))
    .expect("known zero retry counts remain an honest no-retry observation");

    let proposal = result.proposal.expect("technical errors qualify for draft");
    let overlap = &proposal.proposed_artifact["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["status"], "suppressed_below_minimum_support");
    assert_eq!(
        overlap["retry_positive_cases_with_known_error_status"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["retry_and_technical_error_cases"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["error_rate_within_retry_positive_known_error_status_basis_points"],
        serde_json::Value::Null
    );
    assert!(
        overlap["interpretation"]
            .as_str()
            .unwrap()
            .contains("support")
    );
    assert!(
        !overlap["interpretation"]
            .as_str()
            .unwrap()
            .contains("no retry")
    );
}

#[test]
fn retry_error_overlap_requires_complete_tool_call_counts_within_each_case() {
    let mut events = Vec::new();
    let mut known_zero = event(1, 1, Some(true), None, None, None);
    known_zero.retry_count = Some(0);
    events.push(known_zero);
    events.push(event(1, 2, Some(true), None, None, None));
    for case_ordinal in 2..=6 {
        let mut observed = event(case_ordinal, 1, Some(true), None, None, None);
        observed.retry_count = Some(0);
        events.push(observed);
    }
    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-error-overlap-mixed-tool-call-coverage",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=6).collect(),
        0,
        events,
    ))
    .expect("partial ToolCall retry counts do not block proposal generation");

    let proposal = result.proposal.expect("technical errors qualify for draft");
    let overlap = &proposal.proposed_artifact["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["status"], "insufficient_retry_status_coverage");
    assert_eq!(
        overlap["retry_positive_cases_with_known_error_status"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["retry_and_technical_error_cases"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["error_rate_within_retry_positive_known_error_status_basis_points"],
        serde_json::Value::Null
    );
}

#[test]
fn retry_error_overlap_keeps_incomplete_positive_tool_call_case_out_of_denominator() {
    let mut events = Vec::new();
    let mut known_positive = event(1, 1, Some(true), None, None, None);
    known_positive.retry_count = Some(1);
    events.push(known_positive);
    events.push(event(1, 2, Some(true), None, None, None));
    for case_ordinal in 2..=6 {
        let mut observed = event(case_ordinal, 1, Some(true), None, None, None);
        observed.retry_count = Some(0);
        events.push(observed);
    }
    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-error-overlap-positive-incomplete-tool-call-case",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=6).collect(),
        0,
        events,
    ))
    .expect("known positive evidence does not hide incomplete call coverage");

    let proposal = result.proposal.expect("technical errors qualify for draft");
    let overlap = &proposal.proposed_artifact["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["status"], "insufficient_retry_status_coverage");
    assert_eq!(
        overlap["retry_positive_cases_with_known_error_status"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["retry_and_technical_error_cases"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["error_rate_within_retry_positive_known_error_status_basis_points"],
        serde_json::Value::Null
    );
}

#[test]
fn retry_error_overlap_does_not_treat_cases_without_tool_calls_as_no_retries() {
    let events = (1..=6)
        .map(|case_ordinal| {
            let mut observed = event(case_ordinal, 1, Some(true), None, None, None);
            observed.event_kind = "turn".into();
            observed
        })
        .collect();
    let result = run_local_simulation(LocalRunInput::new(
        LocalRunMetadata::new(
            "run-retry-error-overlap-no-tool-calls",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=6).collect(),
        0,
        events,
    ))
    .expect("lack of ToolCall observations does not block proposal generation");

    let proposal = result.proposal.expect("technical errors qualify for draft");
    let overlap = &proposal.proposed_artifact["observed_evidence"]["retry_error_overlap"];
    assert_eq!(overlap["status"], "insufficient_retry_status_coverage");
    assert_eq!(
        overlap["retry_positive_cases_with_known_error_status"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["retry_and_technical_error_cases"],
        serde_json::Value::Null
    );
    assert_eq!(
        overlap["error_rate_within_retry_positive_known_error_status_basis_points"],
        serde_json::Value::Null
    );
}

#[test]
fn local_simulation_runs_detection_to_proposal_without_claiming_native_execution_or_lift() {
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "run-local-01",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        vec![1, 2, 3, 4, 5],
        0,
        vec![
            event(
                1,
                1,
                Some(true),
                Some("payments"),
                Some("tree"),
                Some("status_lookup"),
            ),
            event(
                2,
                1,
                Some(false),
                Some("payments"),
                Some("tree"),
                Some("status_lookup"),
            ),
            event(
                3,
                1,
                Some(true),
                Some("payments"),
                Some("agent_l1"),
                Some("ledger_read"),
            ),
            event(4, 1, None, Some("cards"), Some("copilot"), None),
        ],
    );

    let result = run_local_simulation(input).expect("local run completes");

    assert_eq!(result.execution_mode, "local_simulation");
    assert_eq!(result.observed_cutoff_rfc3339, "2026-08-01T00:00:03Z");
    assert!(
        result
            .events
            .iter()
            .all(|event| event.observed_cutoff_rfc3339 == result.observed_cutoff_rfc3339)
    );
    assert_eq!(result.signal.as_ref().unwrap().numerator, 2);
    assert_eq!(result.signal.as_ref().unwrap().denominator, 3);
    assert_eq!(result.signal.as_ref().unwrap().missing, 2);
    assert_eq!(result.candidates.len(), 3);
    assert_eq!(result.verification_status.as_deref(), Some("uncertain"));
    assert_eq!(
        result.proposal.as_ref().unwrap().execution_status,
        "not_executed"
    );
    assert_eq!(
        result.proposal.as_ref().unwrap().status,
        "simulated_unverified"
    );
    assert!(
        result
            .proposal
            .as_ref()
            .unwrap()
            .hypothesis
            .contains("payments")
    );
    assert_eq!(result.evaluation.as_ref().unwrap().status, "simulated");
    assert!(
        !result
            .evaluation
            .as_ref()
            .unwrap()
            .claims_business_improvement
    );
    assert_eq!(result.formal_route, "do_nothing");
    assert!(
        result
            .events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );

    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains("\"label\":"));
    assert!(!serialized.contains("customer_id"));
    assert!(!serialized.contains("case_id"));
}

#[test]
fn recurring_opaque_copilot_query_across_cases_creates_only_a_simulated_candidate() {
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "run-recurring-query",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=21).collect(),
        2,
        vec![event(
            21,
            1,
            Some(false),
            Some("payments"),
            Some("tree"),
            Some("status_lookup"),
        )],
    )
    .with_queries(
        (1..=20)
            .map(|case_ordinal| {
                LocalObservedQuery::new(
                    case_ordinal,
                    format!("sha256_{}", "a".repeat(56)),
                    "2026-08-01T00:00:01Z",
                )
            })
            .chain([
                LocalObservedQuery::new(
                    1,
                    format!("sha256_{}", "a".repeat(56)),
                    "2026-08-01T00:00:02Z",
                ),
                LocalObservedQuery::new(
                    21,
                    format!("sha256_{}", "b".repeat(56)),
                    "2026-08-01T00:00:01Z",
                ),
            ])
            .collect(),
    );

    let result = run_local_simulation(input).expect("recurrence detector completes");

    let signal = result
        .signal
        .as_ref()
        .expect("recurrence signal is reported");
    assert_eq!(signal.metric_id, "e0_recurring_copilot_query_cases");
    assert_eq!(
        signal.numerator, 20,
        "support counts distinct cases, not rows"
    );
    assert_eq!(signal.denominator, 21);
    assert_eq!(signal.minimum_support, 20);
    assert_eq!(
        signal.detector_policy_id,
        "e0_recurring_copilot_query_support_v1"
    );
    assert_eq!(signal.detector_policy_version, 1);
    assert!(signal.pattern_ref.is_some());
    assert_eq!(
        result.signals.len(),
        3,
        "all available detectors remain visible"
    );
    assert!(
        result
            .signals
            .iter()
            .any(|signal| signal.metric_id == "e0_tool_retry_case_rate")
    );
    assert!(
        result.signals.iter().any(|signal| {
            signal.metric_id == "e0_technical_error_rate" && signal.numerator == 0
        })
    );
    assert_eq!(result.primary_signal_policy, "local_primary_signal_v3");
    assert_eq!(result.terminal_status, "complete_simulated");
    assert_eq!(result.formal_route, "do_nothing");
    assert!(!result.candidates.is_empty());
    let proposal = result
        .proposal
        .as_ref()
        .expect("proposal draft is simulated");
    assert_eq!(proposal.status, "simulated_unverified");
    assert_eq!(proposal.execution_status, "not_executed");
    assert!(proposal.hypothesis.contains("copilot query"));
    assert_eq!(
        proposal.proposed_artifact["observed_evidence"]["primary_signal_policy"],
        "local_primary_signal_v3"
    );
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains(&format!("sha256_{}", "a".repeat(56))));
    assert!(!serialized.contains(&format!("sha256_{}", "b".repeat(56))));
}

#[test]
fn direct_technical_failure_is_primary_without_hiding_other_qualifying_signals() {
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "run-both-signals",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:abababababababababababababababababababababababababababababababab",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        (1..=21).collect(),
        0,
        vec![event(
            1,
            1,
            Some(true),
            Some("payments"),
            Some("tree"),
            Some("status_lookup"),
        )],
    )
    .with_queries(
        (1..=20)
            .map(|case_ordinal| {
                LocalObservedQuery::new(
                    case_ordinal,
                    format!("sha256_{}", "d".repeat(56)),
                    "2026-08-01T00:00:01Z",
                )
            })
            .collect(),
    );

    let result = run_local_simulation(input.clone()).expect("both metrics qualify");

    assert_eq!(result.primary_signal_policy, "local_primary_signal_v3");
    assert_eq!(
        result.signal.as_ref().unwrap().metric_id,
        "e0_technical_error_rate"
    );
    assert!(result.signals.iter().any(|signal| signal.metric_id
        == "e0_recurring_copilot_query_cases"
        && signal.numerator == 20));
    let portfolio = result
        .local_simulation_portfolio
        .as_ref()
        .expect("E0 local runs expose a simulator-only signal portfolio");
    assert_eq!(portfolio.source_family, "e0");
    assert_eq!(portfolio.authority, "simulator_only");
    assert_eq!(portfolio.status, "candidates_ready");
    assert_eq!(
        portfolio.primary_signal_digest.as_deref(),
        result.signal.as_ref().map(|signal| signal.digest.as_str())
    );
    assert_eq!(
        portfolio
            .candidate_signal_digests
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            result.signals[0].digest.as_str(),
            result.signals[2].digest.as_str()
        ]
    );
    assert_eq!(
        portfolio
            .candidate_signal_digests
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        portfolio.candidate_signal_digests.len()
    );
    assert!(portfolio.dispositions.iter().all(|item| matches!(
        item.metric_id.as_str(),
        "e0_technical_error_rate" | "e0_tool_retry_case_rate" | "e0_recurring_copilot_query_cases"
    )));
    assert_eq!(
        portfolio
            .dispositions
            .iter()
            .map(|item| item.metric_id.as_str())
            .collect::<Vec<_>>(),
        result
            .signals
            .iter()
            .map(|item| item.metric_id.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        portfolio.dispositions[0].state,
        "candidate_for_simulated_investigation"
    );
    assert_eq!(
        portfolio.dispositions[0].signal_digest,
        Some(result.signals[0].digest.clone())
    );
    assert_eq!(portfolio.dispositions[1].state, "insufficient_evidence");
    assert_eq!(portfolio.dispositions[1].reason, "no_known_denominator");
    assert_eq!(
        portfolio.dispositions[2].state,
        "candidate_for_simulated_investigation"
    );
    assert_eq!(
        result.proposal.as_ref().unwrap().evidence.metric_id,
        "e0_technical_error_rate"
    );
    let portfolio_event = result
        .events
        .iter()
        .find(|event| event.stage == "signal_portfolio")
        .expect("portfolio timeline summary is present");
    assert_eq!(portfolio_event.status, "simulator_only");
    assert!(portfolio_event.detail.contains("candidates=2"));
    assert!(!portfolio_event.detail.contains(&result.signals[0].digest));
    assert!(!portfolio_event.detail.contains("numerator"));
    let repeated = run_local_simulation(input).expect("identical input is repeatable");
    assert_eq!(
        result
            .local_simulation_portfolio
            .as_ref()
            .unwrap()
            .dispositions
            .iter()
            .map(|item| item.metric_id.as_str())
            .collect::<Vec<_>>(),
        repeated
            .local_simulation_portfolio
            .as_ref()
            .unwrap()
            .dispositions
            .iter()
            .map(|item| item.metric_id.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn absent_query_table_is_reported_as_unavailable_not_zero_recurrence() {
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "run-no-query-table",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        vec![1, 2],
        0,
        vec![event(
            1,
            1,
            Some(true),
            Some("payments"),
            Some("tree"),
            Some("status_lookup"),
        )],
    );

    let result =
        run_local_simulation(input).expect("available technical metric remains measurable");

    assert_eq!(
        result.recurrence_measurement_status,
        "source_table_unavailable"
    );
    assert!(
        !result
            .signals
            .iter()
            .any(|signal| signal.metric_id == "e0_recurring_copilot_query_cases")
    );
    let portfolio = result.local_simulation_portfolio.as_ref().unwrap();
    assert_eq!(portfolio.status, "candidates_ready");
    assert!(portfolio.dispositions.iter().any(|item| {
        item.metric_id == "e0_recurring_copilot_query_cases"
            && item.signal_digest.is_none()
            && item.state == "unavailable"
            && item.reason == "source_table_unavailable"
    }));
}

#[test]
fn recurring_query_below_versioned_support_threshold_remains_visible_but_noops() {
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "run-recurring-query-below-threshold",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        vec![1, 2, 3, 4],
        0,
        Vec::new(),
    )
    .with_queries(
        (1..=4)
            .map(|case_ordinal| {
                LocalObservedQuery::new(
                    case_ordinal,
                    format!("sha256_{}", "c".repeat(56)),
                    "2026-08-01T00:00:01Z",
                )
            })
            .collect(),
    );

    let result = run_local_simulation(input).expect("low-support pattern is reported");

    let recurrence = result
        .signals
        .iter()
        .find(|signal| signal.metric_id == "e0_recurring_copilot_query_cases")
        .expect("recurrence metric remains visible");
    assert_eq!(recurrence.numerator, 4);
    assert_eq!(recurrence.minimum_support, 20);
    assert_eq!(result.terminal_status, "complete_no_opportunity");
    assert!(result.candidates.is_empty());
    assert!(result.proposal.is_none());
}

#[test]
fn zero_positive_support_does_not_create_a_candidate_or_proposal() {
    let mut first_event = event(
        1,
        1,
        Some(false),
        Some("payments"),
        Some("tree"),
        Some("status_lookup"),
    );
    first_event.retry_count = Some(0);
    let mut second_event = event(
        2,
        1,
        Some(false),
        Some("cards"),
        Some("tree"),
        Some("status_lookup"),
    );
    second_event.retry_count = Some(0);
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "run-local-no-opportunity",
            "pulso_local",
            LocalSourceKind::E0,
            "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            snapshot(),
            1_785_542_403,
            "2026-08-01T00:00:03Z",
        ),
        vec![1, 2],
        1,
        vec![first_event, second_event],
    )
    .with_queries(vec![
        LocalObservedQuery::new(
            1,
            format!("sha256_{}", "a".repeat(56)),
            "2026-08-01T00:00:01Z",
        ),
        LocalObservedQuery::new(
            2,
            format!("sha256_{}", "b".repeat(56)),
            "2026-08-01T00:00:02Z",
        ),
    ]);

    let result = run_local_simulation(input).expect("detector reports no opportunity");

    assert_eq!(result.terminal_status, "complete_no_opportunity");
    assert_eq!(result.formal_route, "do_nothing");
    assert_eq!(result.signal.as_ref().unwrap().numerator, 0);
    let portfolio = result.local_simulation_portfolio.as_ref().unwrap();
    assert_eq!(portfolio.status, "no_qualifying_signals");
    assert!(portfolio.candidate_signal_digests.is_empty());
    assert!(
        portfolio
            .dispositions
            .iter()
            .all(|item| item.state == "not_qualified")
    );
    assert!(result.candidates.is_empty());
    assert!(result.proposal.is_none());
    assert!(result.evaluation.is_none());
    assert!(
        result
            .events
            .iter()
            .any(|event| { event.stage == "scout" && event.status == "no_opportunity" })
    );
    assert!(
        !result
            .events
            .iter()
            .any(|event| event.stage == "improvement_draft")
    );
}

#[test]
fn source_without_an_allowlisted_signal_is_reported_not_fabricated() {
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "run-local-02",
            "pulso_local",
            LocalSourceKind::OriginalBank,
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            snapshot(),
            1_785_542_401,
            "2026-08-01T00:00:01Z",
        ),
        vec![1, 2],
        0,
        vec![
            event(1, 1, None, Some("cards"), Some("tree"), None),
            event(2, 1, None, Some("cards"), Some("tree"), None),
        ],
    );

    let result = run_local_simulation(input).expect("unsupported source still yields a run");

    assert!(result.signal.is_none());
    assert!(result.local_simulation_portfolio.is_none());
    assert_eq!(result.terminal_status, "unsupported_source");
    assert!(result.candidates.is_empty());
    assert!(result.proposal.is_none());
    assert_eq!(result.events.last().unwrap().stage, "run_completed");
}

#[test]
fn original_snapshot_contact_projection_is_reported_without_claiming_a_signal() {
    use improvement_engine_core::local_simulation::{
        LocalContactVolumeCell, LocalContactVolumeProjection,
    };

    let projection = LocalContactVolumeProjection::new(
        1,
        5,
        5,
        vec![LocalContactVolumeCell::new("complaint", "phone", 5)],
    )
    .unwrap();
    let input = LocalRunInput::new(
        LocalRunMetadata::new(
            "run-contact-snapshot",
            "pulso_local",
            LocalSourceKind::OriginalBank,
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            snapshot(),
            1_785_542_401,
            "2026-08-01T00:00:01Z",
        ),
        Vec::new(),
        0,
        Vec::new(),
    )
    .with_contact_volume_projection(projection);

    let result = run_local_simulation(input).unwrap();

    assert_eq!(result.terminal_status, "snapshot_projection_complete");
    assert!(result.signal.is_none());
    assert!(result.candidates.is_empty());
    assert!(result.proposal.is_none());
    let reported = result.contact_volume_projection.as_ref().unwrap();
    assert_eq!(reported.semantics(), "snapshot_extract_counts");
    assert_eq!(reported.included_record_count(), 5);
    assert_eq!(reported.cells()[0].reason_category(), "complaint");
    assert_eq!(reported.cells()[0].channel(), "phone");
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("customer_id")
    );
}

#[test]
fn contact_projection_enforces_versioned_k_bounds_at_the_core_boundary() {
    use improvement_engine_core::local_simulation::LocalContactVolumeProjection;

    for minimum_cell_count in [1, 4, 6, 10_001] {
        assert!(LocalContactVolumeProjection::new(1, minimum_cell_count, 0, Vec::new(),).is_err());
    }
    assert!(LocalContactVolumeProjection::new(2, 5, 0, Vec::new(),).is_err());
}
