//! STP1 intent (validation) step (`semantics: claude-standin`).
use crate::StepError;

pub fn run(_input: &str) -> Result<String, StepError> {
    Err(StepError::Runner("not implemented".into()))
}
