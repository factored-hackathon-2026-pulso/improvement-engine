use improvement_engine_core::ArtifactReference;
use improvement_engine_core::local_simulation::{
    LocalObservedEvent, LocalRunInput, LocalSourceKind, run_local_simulation,
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
        "run-local-01",
        "pulso_local",
        LocalSourceKind::E0,
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        snapshot(),
        1_785_542_403,
        "2026-08-01T00:00:03Z",
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
fn zero_positive_support_does_not_create_a_candidate_or_proposal() {
    let input = LocalRunInput::new(
        "run-local-no-opportunity",
        "pulso_local",
        LocalSourceKind::E0,
        "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        snapshot(),
        1_785_542_403,
        "2026-08-01T00:00:03Z",
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
        "run-local-02",
        "pulso_local",
        LocalSourceKind::OriginalBank,
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        snapshot(),
        1_785_542_401,
        "2026-08-01T00:00:01Z",
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
