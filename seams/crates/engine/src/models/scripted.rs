//! `Scripted`: the current behaviour of the thread, as a port. No model runs; every answer is a fixed value.
use super::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest, Role};
use serde_json::json;

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
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        let content = match req.role {
            Role::Scout => {
                let sid = req.payload.pointer("/inputs/signal_id").and_then(|v| v.as_str()).unwrap_or("");
                json!({"hypotheses": [{"id": "h_1", "signal_id": sid, "claimed_rate": self.claimed_rate}]})
            }
            Role::Verifier => json!({"verdict": "agree"}),
            Role::Builder => json!({"proposal": {"kind": "prompt", "op": self.op, "target_ref": "prompt:resumen_radicado@1",
                "new_ref": "prompt:resumen_radicado@2", "mechanism": "shorter closing reply"}}),
        };
        Ok(ModelAnswer { content, model_id: self.model_id(), label: Label::Scripted })
    }
}
