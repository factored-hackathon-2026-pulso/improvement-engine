//! Step stand-ins behind the FRZ0 step schema (contracts/engine-steps).
//! One module per lane so the STP1, CMP and GSI work packages never touch the same file:
//! `sensor`, `recompute`, `intent` (STP1), `compile` (CMP), `gate` (GSI).
//! Every output carries its semantics label (`claude-standin`); none of these claims Codex semantics.

pub mod compile;
pub mod gate;
pub mod intent;
pub mod recompute;
pub mod sensor;

/// Label stamped on every stand-in output.
pub const SEMANTICS: &str = "claude-standin";

/// Error shared by every step stand-in (STP1 adds it; CMP and GSI reuse it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepError {
    /// Input is not valid JSON or violates the step input schema.
    Invalid(String),
    /// A referenced file, directory or executable could not be used.
    Io(String),
    /// The wrapped runner failed or produced an unusable result.
    Runner(String),
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepError::Invalid(m) => write!(f, "invalid: {m}"),
            StepError::Io(m) => write!(f, "io: {m}"),
            StepError::Runner(m) => write!(f, "runner: {m}"),
        }
    }
}

impl std::error::Error for StepError {}
