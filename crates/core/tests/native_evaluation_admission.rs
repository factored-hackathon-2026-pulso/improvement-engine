//! Public contract for the fail-closed U19-0 admission boundary.

use improvement_engine_core::native_evaluation::{
    NativeEvaluationAdmissionError, NativeEvaluationDispatchState, NativeEvaluationRequest,
};

#[test]
fn public_surface_exposes_only_opaque_request_and_truthful_dispatch_states() {
    let _: Option<NativeEvaluationRequest> = None;
    assert_eq!(
        NativeEvaluationDispatchState::NotDispatched.as_str(),
        "not_dispatched"
    );
    assert_eq!(
        NativeEvaluationDispatchState::UnknownAfterDispatch.as_str(),
        "unknown_after_dispatch"
    );
    assert_eq!(
        NativeEvaluationAdmissionError::DependencyUnavailable.as_str(),
        "dependency_unavailable"
    );
}
