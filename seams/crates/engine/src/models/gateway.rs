//! `Gateway`: HTTP client to an llm-gateway-compatible endpoint (`POST /v1/chat/completions`, messages
//! `[system, user]`, the user message being the canonical JSON of the treated agent input dict; the contract the
//! roleplay shim serves too). Plain HTTP over `std::net` (the gateway is a sidecar on the same host; there is no TLS here).
//!
//! Disabled unless explicitly configured. Env: `PULSO_MODEL_GATEWAY=enabled` (exact value), `PULSO_GATEWAY_ADDR`
//! (`host:port`), `PULSO_GATEWAY_MODEL`, optional `PULSO_GATEWAY_KEY` (bearer; never logged or recorded) and
//! `PULSO_GATEWAY_KIND` = `gateway` (default) | `local-model` (an endpoint known to serve a local model: labelled
//! `local-model`, never `real`). Before any byte is sent the request passes `guard`: E0/original data is refused by data
//! class and the payload must pass the TPS scan. A refusal, an outage or an unusable answer is an error, never a fallback.
use super::roleplay::canonical;
use super::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest, guard};
use core_client::http::{HttpError, request};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone)]
pub struct GatewayConfig {
    pub addr: String,
    pub model: String,
    pub key: Option<String>,
    pub label: Label,
    pub timeout: Duration,
}

pub struct Gateway {
    config: Option<GatewayConfig>,
}

impl Gateway {
    /// A gateway that refuses every call.
    pub fn disabled() -> Gateway {
        Gateway { config: None }
    }

    /// `Ok(disabled)` unless `PULSO_MODEL_GATEWAY` is exactly `enabled`; an enabled but incomplete configuration is an error.
    pub fn from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<Gateway, String> {
        if get("PULSO_MODEL_GATEWAY").as_deref() != Some("enabled") {
            return Ok(Gateway::disabled());
        }
        let need = |k: &str| get(k).filter(|v| !v.trim().is_empty()).ok_or_else(|| format!("PULSO_MODEL_GATEWAY=enabled needs {k}"));
        let label = match get("PULSO_GATEWAY_KIND").as_deref() {
            None | Some("gateway") => Label::Gateway,
            Some("local-model") => Label::LocalModel,
            Some(o) => return Err(format!("PULSO_GATEWAY_KIND {o:?} is not gateway|local-model")),
        };
        Ok(Gateway {
            config: Some(GatewayConfig { addr: need("PULSO_GATEWAY_ADDR")?, model: need("PULSO_GATEWAY_MODEL")?, key: get("PULSO_GATEWAY_KEY").filter(|k| !k.is_empty()), label, timeout: Duration::from_secs(60) }),
        })
    }
}

impl ModelPort for Gateway {
    fn label(&self) -> Label {
        self.config.as_ref().map_or(Label::Gateway, |c| c.label)
    }
    fn model_id(&self) -> String {
        self.config.as_ref().map_or_else(|| "gateway-disabled".into(), |c| c.model.clone())
    }
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        let Some(c) = &self.config else {
            return Err(ModelError::Refused("gateway_disabled: set PULSO_MODEL_GATEWAY=enabled with PULSO_GATEWAY_ADDR and PULSO_GATEWAY_MODEL".into()));
        };
        guard(req)?;
        let body = json!({"model": c.model, "messages": [{"role": "system", "content": req.system}, {"role": "user", "content": canonical(&req.payload)}]});
        let headers: Vec<(&str, String)> = c.key.iter().map(|k| ("Authorization", format!("Bearer {k}"))).collect();
        let r = request(&c.addr, "POST", "/v1/chat/completions", &headers, Some(body.to_string().as_bytes()), c.timeout).map_err(|e| match e {
            HttpError::Connect(m) => ModelError::Unavailable(format!("gateway_unreachable: {m}")),
            HttpError::Io(m) | HttpError::Protocol(m) => ModelError::Unavailable(format!("gateway_io: {m}")),
        })?;
        let doc: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        match r.status {
            200 => {}
            422 if doc["error"]["type"] == "treated_payload_rejected" => return Err(ModelError::Refused("remote_tps: the gateway scanner rejected the payload".into())),
            s => return Err(ModelError::Unavailable(format!("gateway_http_{s}: {}", doc["error"]["type"].as_str().unwrap_or("no error type")))),
        }
        let text = doc.pointer("/choices/0/message/content").and_then(Value::as_str).ok_or_else(|| ModelError::Invalid("gateway answer has no choices[0].message.content".into()))?;
        let content: Value = serde_json::from_str(text).map_err(|_| ModelError::Invalid("gateway message content is not JSON".into()))?;
        if !content.is_object() {
            return Err(ModelError::Invalid("gateway message content is not a JSON object".into()));
        }
        let model_id = doc["model"].as_str().filter(|m| !m.is_empty()).unwrap_or(&c.model).to_string();
        Ok(ModelAnswer { content, model_id, label: c.label })
    }
}
