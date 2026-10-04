//! CMP stand-in (stub: RED).

#[derive(Debug, PartialEq, Eq)]
pub struct CompileError(pub String);

pub fn run(_input: &str) -> Result<String, CompileError> {
    Err(CompileError("not implemented".into()))
}

pub struct World;
impl World {
    pub fn seeded_base() -> World {
        World
    }
}

pub fn run_with(_input: &str, _w: &World, _dry: Option<&dyn Fn(&[String]) -> String>) -> Result<String, CompileError> {
    Err(CompileError("not implemented".into()))
}
