//! Scripted `ModelPort`s for offline tests and for the plumbing demo of the CLI. Labelled `scripted`: never `real`.
use engine::models::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest, Role};
use serde_json::Value;

type Answer = dyn Fn(&ModelRequest) -> Result<Value, ModelError>;

/// A port whose answers come from a closure over the request (so a test can read the anchor menu and answer from it).
pub struct FnPort {
    pub label: Label,
    pub model: String,
    pub answer: Box<Answer>,
}

impl FnPort {
    pub fn scripted(model: &str, answer: impl Fn(&ModelRequest) -> Result<Value, ModelError> + 'static) -> FnPort {
        FnPort { label: Label::Scripted, model: model.into(), answer: Box::new(answer) }
    }
    /// Same port, but labelled as the hosted gateway (used only to exercise the opt-in rule without a network).
    pub fn pretending_gateway(model: &str, answer: impl Fn(&ModelRequest) -> Result<Value, ModelError> + 'static) -> FnPort {
        FnPort { label: Label::Gateway, model: model.into(), answer: Box::new(answer) }
    }
}

impl ModelPort for FnPort {
    fn label(&self) -> Label {
        self.label
    }
    fn model_id(&self) -> String {
        self.model.clone()
    }
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        // the same scan the real ports run, so a scripted test also proves the payload is TPS-clean
        let scan = engine::models::tps::scan_payload(&req.payload, engine::models::tps::DEFAULT_K, &req.registry);
        if !scan.ok {
            return Err(ModelError::Refused(format!("tps: {}", scan.violations.join("; "))));
        }
        (self.answer)(req).map(|content| ModelAnswer { content, model_id: self.model.clone(), label: self.label })
    }
}

/// The anchor menu a Builder request carries, as `(anchor id, text)` pairs, read back from the tool descriptions.
pub fn menu_of(req: &ModelRequest) -> Vec<(String, String)> {
    req.payload["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["tool"].as_str().is_some_and(|n| n.starts_with("pulso/anchors_")))
        .flat_map(|t| t["description"].as_str().unwrap_or("").lines().filter_map(|l| l.split_once(" | ")).map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>())
        .collect()
}

pub fn role_of(req: &ModelRequest) -> Role {
    req.role
}

/// A synthetic M1 cell table (baseline 20%, Tecnico/Phone planted at 45% in both halves) as the aggregator's ndjson, for demos and
/// tests. Invented numbers: the data class is `synthetic`.
pub fn synthetic_cells_ndjson() -> String {
    let mut rows = vec![];
    for r in ["Queja", "Tecnico", "Comercial", "Retencion", "Transaccional", "Producto"] {
        for c in ["Phone", "Chat"] {
            for (half, den) in [("discovery", 6000i64), ("holdout", 4000i64)] {
                let permille = if r == "Tecnico" && c == "Phone" { 450 } else { 200 };
                rows.push(format!(
                    "{{\"metric\":\"M1\",\"dims\":{{\"reason_category\":\"{r}\",\"channel\":\"{c}\"}},\"half\":\"{half}\",\"numerator\":{},\"denominator\":{den}}}",
                    den * permille / 1000
                ));
            }
        }
    }
    rows.join("\n")
}
