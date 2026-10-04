//! Tests are compiled as an external consumer of the crate's public surface.

use improvement_engine_core::facade_steps::provisional::PlatformSourceReader;
use improvement_engine_core::{
    ArmBinding, ArmObservation, InfrastructureFailure, PairError, PairPlan, PairVerdict,
    PlatformColumn, PlatformRelation, PlatformSourceReadPlan, PlatformSourceText,
    PlatformTreatedText,
};

#[test]
fn consumer_can_name_the_frozen_composite_step_types() {
    let _pair_plan: Option<PairPlan> = None;
    let _arm_observation: Option<ArmObservation> = None;
    let _arm_binding: Option<ArmBinding> = None;
    let _pair_verdict: Option<PairVerdict> = None;
    let _infrastructure_failure: Option<InfrastructureFailure> = None;
    let _pair_error: Option<PairError> = None;
    let _source_read_plan = PlatformSourceReadPlan::platform_live();
    let _relation = PlatformRelation::Cases;
    let _column = PlatformColumn::CaseId;
    let _raw_text: Option<PlatformSourceText> = None;
    let _treated_text: Option<PlatformTreatedText> = None;
    let _reader: Option<&dyn PlatformSourceReader<Error = std::convert::Infallible>> = None;
}
