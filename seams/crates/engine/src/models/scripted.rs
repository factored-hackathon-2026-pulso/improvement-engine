//! `Scripted`: the current behaviour of the thread, as a port. No model runs; every answer is a fixed value.
use super::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest};

pub struct Scripted {
    /// What the scripted scout claims (the lab value is 0.30).
    pub claimed_rate: f64,
    /// The operation the scripted builder proposes on the prompt (`replace` is the only supported one).
    pub op: String,
}

impl Scripted {
    pub fn new() -> Scripted {
        Scripted { claimed_rate: 0.3, op: "replace".into() }
    }
}

impl Default for Scripted {
    fn default() -> Self {
        Scripted::new()
    }
}

impl ModelPort for Scripted {
    fn label(&self) -> Label {
        Label::Scripted
    }
    fn model_id(&self) -> String {
        "scripted-v1".into()
    }
    fn call(&self, _req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        todo!("RED")
    }
}
