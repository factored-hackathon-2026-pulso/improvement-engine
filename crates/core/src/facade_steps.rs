//! Explicitly curated public facades for composite engine steps.
//!
//! These re-exports give downstream crates a small, discoverable entry point
//! without changing the underlying domain modules or their trust boundaries.
//! A public type here is not evidence that an observation came from a real
//! sandbox or that a business outcome improved.
//! `PlatformSourceReader` remains under [`provisional`] because its result
//! shape must change before a usable row-returning source adapter can land.
//!
//! The facade must not widen a sealed treatment constructor:
//!
//! ```compile_fail
//! use improvement_engine_core::PlatformTreatedText;
//! let _ = PlatformTreatedText::from_authoritative_treatment;
//! ```

pub use crate::paired_scenario::{
    ArmBinding, ArmObservation, InfrastructureFailure, PairError, PairPlan, PairVerdict,
};
pub use crate::platform_source_policy::{
    PlatformColumn, PlatformRelation, PlatformSourceReadPlan, PlatformSourceText,
    PlatformTreatedText,
};

/// Public, explicitly provisional exports whose contract is expected to evolve.
pub mod provisional {
    pub use crate::platform_source_policy::PlatformSourceReader;
}
