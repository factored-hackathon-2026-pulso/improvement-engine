//! The default `ModelPort` of `pulso run`: scripted, and labelled so. No model runs; every answer is a fixed rule.
//!
//! Unlike the single-value `engine::models::Scripted` (which claims 0.30 for every signal), the scout here copies the aggregate
//! rate of the observation row it is given, so a signal measured at 54/321 is claimed at 0.17 and the independent recompute
//! (numerator over count, two decimals) corroborates it. That makes the verdict depend on the gate and the Core port, not on a
//! claim that was always going to be refuted. It is still a scripted value: the label stays `scripted`, never `real`.
use engine::models::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest, Role};
use serde_json::json;

pub const MODEL_ID: &str = "scripted-observing-v1";

#[derive(Default)]
pub struct ObservingScripted;

impl ModelPort for ObservingScripted {
    fn label(&self) -> Label {
        Label::Scripted
    }
    fn model_id(&self) -> String {
        MODEL_ID.into()
    }
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        let content = match req.role {
            Role::Scout => {
                let sid = req.payload.pointer("/inputs/signal_id").and_then(|v| v.as_str()).unwrap_or("");
                let rate = req
                    .payload
                    .pointer("/observations/0/result/rows/0/rate")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| ModelError::Invalid("the observation row has no rate to copy (suppressed or missing)".into()))?;
                json!({"hypotheses": [{"id": "h_1", "signal_id": sid, "claimed_rate": rate}]})
            }
            Role::Verifier => json!({"verdict": "agree"}),
            Role::Builder => json!({"proposal": {"kind": "prompt", "op": "replace", "target_ref": "prompt:resumen_radicado@1", "new_ref": "prompt:resumen_radicado@2", "mechanism": "shorter closing reply"}}),
        };
        Ok(ModelAnswer { content, model_id: MODEL_ID.into(), label: Label::Scripted })
    }
}
