//! `LlmGateway`: HTTP client for the REAL llm-gateway contract (`POST /v1/generate`: `{prompt, inputs, schema?, profile, labels?}`
//! answered by `{output, model, tokens_in, tokens_out, cost_usd, usage_known}`). The older `Gateway` port speaks the
//! chat-completions shape of the roleplay shim; this one speaks what `llm-gateway` serves. Plain HTTP over `std::net`
//! (loopback or private hosts only, same rule as `Gateway`).
//!
//! Disabled unless `PULSO_LLM_GATEWAY=enabled` (exact value). Env: `PULSO_LLM_GATEWAY_ADDR` (`host:port`),
//! `PULSO_LLM_GATEWAY_KEY` (bearer; never logged, recorded or part of any Debug output), optional `PULSO_LLM_GATEWAY_MODEL`
//! (default `deepseek/deepseek-v4.1-flash`), `PULSO_LLM_GATEWAY_ALIAS` (default `openrouter`), `PULSO_LLM_GATEWAY_MAX_TOKENS`
//! (default 4000), `PULSO_LLM_GATEWAY_TIMEOUT_S` (default 60), `PULSO_LLM_GATEWAY_PRICE_IN` / `_OUT` (USD per million tokens as
//! decimal strings; defaults are an estimate, the gateway only echoes the cost). The request passes `guard` first (E0/original
//! data refused by class, TPS scan, size cap). `inputs` is the treated agent input dict; the payload's `output_schema`, when
//! present, travels as the gateway response `schema` (prompted mode, validated by the gateway). A refusal, an outage or an
//! unusable answer is an error, never a fallback.
use super::gateway::private_host;
use super::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest, guard};
use core_client::http::{HttpError, request};
use serde_json::{Value, json};
use std::time::Duration;

pub const DEFAULT_MODEL: &str = "deepseek/deepseek-v4.1-flash";

#[derive(Clone)]
struct Config {
    addr: String,
    key: String,
    model: String,
    alias: String,
    max_tokens: u64,
    timeout_s: u64,
    price_in: String,
    price_out: String,
}

pub struct LlmGateway {
    config: Option<Config>,
}

impl LlmGateway {
    pub fn disabled() -> LlmGateway {
        LlmGateway { config: None }
    }

    pub fn from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<LlmGateway, String> {
        if get("PULSO_LLM_GATEWAY").as_deref() != Some("enabled") {
            return Ok(LlmGateway::disabled());
        }
        let need = |k: &str| get(k).filter(|v| !v.trim().is_empty()).ok_or_else(|| format!("PULSO_LLM_GATEWAY=enabled needs {k}"));
        let addr = need("PULSO_LLM_GATEWAY_ADDR")?;
        if !private_host(&addr) && get("PULSO_GATEWAY_ALLOW_REMOTE_PLAINTEXT").as_deref() != Some("yes") {
            return Err(format!("PULSO_LLM_GATEWAY_ADDR {addr:?} is not a loopback or private host and this client has no TLS"));
        }
        let num = |k: &str, d: u64| -> Result<u64, String> { get(k).map_or(Ok(d), |v| v.parse().map_err(|_| format!("{k} is not a number"))) };
        let dec = |k: &str, d: &str| -> Result<String, String> {
            let v = get(k).unwrap_or_else(|| d.to_string());
            if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit() || b == b'.') { Err(format!("{k} is not a decimal string")) } else { Ok(v) }
        };
        Ok(LlmGateway {
            config: Some(Config {
                addr,
                key: need("PULSO_LLM_GATEWAY_KEY")?,
                model: get("PULSO_LLM_GATEWAY_MODEL").filter(|m| !m.is_empty()).unwrap_or_else(|| DEFAULT_MODEL.into()),
                alias: get("PULSO_LLM_GATEWAY_ALIAS").filter(|m| !m.is_empty()).unwrap_or_else(|| "openrouter".into()),
                max_tokens: num("PULSO_LLM_GATEWAY_MAX_TOKENS", 4000)?,
                timeout_s: num("PULSO_LLM_GATEWAY_TIMEOUT_S", 60)?,
                price_in: dec("PULSO_LLM_GATEWAY_PRICE_IN", "0.15")?,
                price_out: dec("PULSO_LLM_GATEWAY_PRICE_OUT", "2.4")?,
            }),
        })
    }
}

impl ModelPort for LlmGateway {
    fn label(&self) -> Label {
        Label::Gateway
    }
    fn model_id(&self) -> String {
        self.config.as_ref().map_or_else(|| "llm-gateway-disabled".into(), |c| c.model.clone())
    }
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        let Some(c) = &self.config else {
            return Err(ModelError::Refused("llm_gateway_disabled: set PULSO_LLM_GATEWAY=enabled with PULSO_LLM_GATEWAY_ADDR and PULSO_LLM_GATEWAY_KEY".into()));
        };
        guard(req)?;
        let mut body = json!({
            "prompt": req.system,
            "inputs": req.payload,
            "profile": {"endpoint_alias": c.alias, "model": c.model, "temperature": 0, "max_tokens": c.max_tokens, "timeout_s": c.timeout_s,
                        "structured": "prompted", "price": {"input_per_mtok": c.price_in, "output_per_mtok": c.price_out}},
            "labels": {"agent": format!("pulso-{}", req.role.as_str())},
        });
        if let Some(schema) = req.payload.get("output_schema").filter(|s| s.is_object()) {
            body["schema"] = schema.clone();
        }
        let headers = [("Authorization", format!("Bearer {}", c.key)), ("Content-Type", "application/json".to_string())];
        let r = request(&c.addr, "POST", "/v1/generate", &headers, Some(body.to_string().as_bytes()), Duration::from_secs(c.timeout_s + 10)).map_err(|e| match e {
            HttpError::Connect(m) => ModelError::Unavailable(format!("gateway_unreachable: {m}")),
            HttpError::Io(m) | HttpError::Protocol(m) => ModelError::Unavailable(format!("gateway_io: {m}")),
        })?;
        let doc: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        let kind = doc.pointer("/error/kind").and_then(Value::as_str).unwrap_or("no_error_kind").to_string();
        match r.status {
            200 => {}
            502 if kind == "invalid_output" => return Err(ModelError::Invalid("gateway_invalid_output: the model answer is not JSON, violates the schema or was truncated".into())),
            s => return Err(ModelError::Unavailable(format!("gateway_http_{s}: {kind}"))),
        }
        let content = doc.get("output").filter(|o| o.is_object()).cloned().ok_or_else(|| ModelError::Invalid("gateway output is not a JSON object".into()))?;
        let model_id = doc["model"].as_str().filter(|m| !m.is_empty()).unwrap_or(&c.model).to_string();
        Ok(ModelAnswer { content, model_id, label: Label::Gateway })
    }
}
