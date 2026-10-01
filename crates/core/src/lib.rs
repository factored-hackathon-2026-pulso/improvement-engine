//! Core vocabulary for the Pulso autonomous improvement service.

pub mod source_validation;

/// Stable identifier used by diagnostics and future service composition.
pub fn service_name() -> &'static str {
    "improvement-engine-core"
}
