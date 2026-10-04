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
