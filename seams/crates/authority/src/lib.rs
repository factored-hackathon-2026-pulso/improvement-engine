//! U21 human-authority state machine STAND-IN. Label: [`LABEL`]. NOT Codex semantics: Codex owns the real
//! U21/U27 in crates/core; this crate mirrors names/transitions only and never depends on it.
//! draft -> evaluating -> gated -> waiting_human -> approved -> published | rejected | expired | revoked.
pub mod flow;
pub mod issuer;
mod machine;

pub use issuer::{HumanIssuer, IssuerError, Op, SimTicket, SimulatedIssuer, Target, Verified};
pub use machine::*;

pub const LABEL: &str = "authority=claude-standin";
