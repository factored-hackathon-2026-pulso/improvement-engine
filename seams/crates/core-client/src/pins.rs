//! Pin constants, generated from `bridge-contract/` and ADR 0012 (see `gen/gen_tables.py`).
//! `tests/pins.rs` fails when they drift from their sources.
pub use crate::generated::{
    AGENT_CORE_SHA, BASE_PATH, CONTRACTS_VERSION, CONTRACT_REVISION, JWT_AUD, JWT_ISS, JWT_MAX_TTL_SECONDS,
    MANIFEST_SHA256,
};

/// Short form used in directory and ADR names (`agent_core@c814c2b`).
pub const AGENT_CORE_SHORT: &str = "c814c2b";
