//! `Gateway`: HTTP client to an llm-gateway-compatible endpoint (`POST /v1/chat/completions`).
use super::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest};

pub struct Gateway;

impl Gateway {
    pub fn from_env(_get: &dyn Fn(&str) -> Option<String>) -> Result<Gateway, String> {
        Ok(Gateway)
    }
}

impl ModelPort for Gateway {
    fn label(&self) -> Label {
        Label::Gateway
    }
    fn model_id(&self) -> String {
        "gateway".into()
    }
    fn call(&self, _req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        Err(ModelError::Unavailable("todo".into()))
    }
}
