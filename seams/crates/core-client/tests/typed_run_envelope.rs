//! First RED of K2: the run envelope of the Python bridge golden must decode into the typed `TaskReceipt`.
mod common;
use common::golden::golden_response;
use core_client::dto::{RunState, TaskReceipt};

#[test]
fn golden_run_envelope_decodes() {
    for (flow, case, status) in [
        ("invoke_scout", "invoke_scout_ok", 200),
        ("invoke_scout", "invoke_scout_replay", 200),
        ("invoke_scout", "read_task_ok", 200),
        ("writer_evaluation", "invoke_evaluate_only", 200),
    ] {
        let body = golden_response(flow, case);
        let r = TaskReceipt::from_response(status, &body).unwrap_or_else(|e| panic!("{flow}/{case}: {e}"));
        assert_eq!(r.state, RunState::TerminalOk, "{case}");
        assert!(r.core_run_id.as_deref().is_some_and(|s| s.starts_with("<core_run_id#")), "{case}");
        assert_eq!(r.task_binding_ref, body["task_binding_ref"].as_str().unwrap());
    }
}
