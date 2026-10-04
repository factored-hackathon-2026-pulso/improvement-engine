//! GSI gate stand-in (RED stub).
#[derive(Debug)]
pub struct GateError(pub String);
pub const LABEL: &str = "";
pub fn run(_input: &str) -> Result<String, GateError> {
    Err(GateError("unimplemented".into()))
}
pub fn canonical_json(_input: &str) -> Result<String, GateError> {
    Err(GateError("unimplemented".into()))
}
