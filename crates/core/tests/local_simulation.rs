use improvement_engine_core::ArtifactReference;
use improvement_engine_core::local_simulation::{
    LocalObservedEvent, LocalObservedQuery, LocalRunInput, LocalRunMetadata, LocalSourceKind,
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
        approval: None,
        signal_code: None,
    }
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
    assert_eq!(result.signals.len(), 2, "both detectors remain visible");
    assert!(
        result.signals.iter().any(|signal| {
            signal.metric_id == "e0_technical_error_rate" && signal.numerator == 0
        })
    );
    assert_eq!(result.primary_signal_policy, "local_primary_signal_v2");
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
        "local_primary_signal_v2"
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

    let result = run_local_simulation(input).expect("both metrics qualify");

    assert_eq!(result.primary_signal_policy, "local_primary_signal_v2");
    assert_eq!(
        result.signal.as_ref().unwrap().metric_id,
        "e0_technical_error_rate"
    );
    assert!(result.signals.iter().any(|signal| signal.metric_id
        == "e0_recurring_copilot_query_cases"
        && signal.numerator == 20));
    assert_eq!(
        result.proposal.as_ref().unwrap().evidence.metric_id,
        "e0_technical_error_rate"
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
            Some(false),
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
        vec![
            event(
                1,
                1,
                Some(false),
                Some("payments"),
                Some("tree"),
                Some("status_lookup"),
            ),
            event(
                2,
                1,
                Some(false),
                Some("cards"),
                Some("tree"),
                Some("status_lookup"),
            ),
        ],
    );

    let result = run_local_simulation(input).expect("detector reports no opportunity");

    assert_eq!(result.terminal_status, "complete_no_opportunity");
    assert_eq!(result.formal_route, "do_nothing");
    assert_eq!(result.signal.as_ref().unwrap().numerator, 0);
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
    assert_eq!(result.terminal_status, "unsupported_source");
    assert!(result.candidates.is_empty());
    assert!(result.proposal.is_none());
    assert_eq!(result.events.last().unwrap().stage, "run_completed");
}
