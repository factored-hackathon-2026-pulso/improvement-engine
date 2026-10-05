//! `LlmGateway`: HTTP client for the REAL llm-gateway contract (`POST /v1/generate`: `{prompt, inputs, schema?, profile, labels?}`
//! answered by `{output, model, tokens_in, tokens_out, cost_usd, usage_known}`). The older `Gateway` port speaks the
//! chat-completions shape of the roleplay shim; this one speaks what `llm-gateway` serves. Plain HTTP over `std::net`
//! (loopback or private hosts only, same rule as `Gateway`).
//!
//! Disabled unless `PULSO_LLM_GATEWAY=enabled` (exact value). Env: `PULSO_LLM_GATEWAY_ADDR` (`host:port`),
//! `PULSO_LLM_GATEWAY_KEY` (bearer; never logged, recorded or part of any Debug output), optional `PULSO_LLM_GATEWAY_MODEL`
//! (default `xiaomi/mimo-v2.6-flash`), `PULSO_LLM_GATEWAY_ALIAS` (default `openrouter`), `PULSO_LLM_GATEWAY_MAX_TOKENS`
//! (default 4000; the model is per PORT: each role has its own instance, hence its own `profile.model` and price in every request), `PULSO_LLM_GATEWAY_TIMEOUT_S` (default 60), `PULSO_LLM_GATEWAY_PRICE_IN` / `_OUT` (USD per million tokens as
//! decimal strings; defaults are an estimate, the gateway only echoes the cost). The request passes `guard` first (E0/original
//! data refused by class, TPS scan, size cap). `inputs` is the treated agent input dict; the payload's `output_schema`, when
//! present, travels as the gateway response `schema` (prompted mode, validated by the gateway). A refusal, an outage or an
//! unusable answer is an error, never a fallback.
use super::gateway::private_host;
use super::extract::extract_json_object;
use super::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest, Usage, guard};
use core_client::http::{HttpError, request};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// How the answer shape is obtained (`PULSO_LLM_GATEWAY_STRUCTURED`).
/// - `prompted`: the role schema travels as the gateway `schema`; the gateway appends it to the prompt and validates the answer
///   (its parser accepts a bare object or ONE wrapping code fence only; prose or reasoning around the JSON is `invalid_output`).
/// - `native`: the same schema as a provider `response_format` (strict), when the provider honours it.
/// - `text`: no `schema` is sent, the gateway returns the raw text and THIS client extracts one JSON object from it (code fences,
///   prose, reasoning blocks, trailing commas) before the role parser validates it strictly. Same acceptance, tolerant packaging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Structured {
    Prompted,
    Native,
    Text,
}

impl Structured {
    pub fn as_str(self) -> &'static str {
        match self {
            Structured::Prompted => "prompted",
            Structured::Native => "native",
            Structured::Text => "text",
        }
    }
}

/// USD per million tokens (in, out) from the OpenRouter list, by model id; any other model gets the generation-tier estimate.
pub fn default_price(model: &str) -> (&'static str, &'static str) {
    match model {
        "xiaomi/mimo-v2.6-pro" => ("0.435", "0.87"),
        "z-ai/glm-5.3-flash" => ("0.15", "0.5"),
        _ => ("0.14", "0.28"),
    }
}

pub const DEFAULT_MODEL: &str = "xiaomi/mimo-v2.6-flash";

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
    structured: Structured,
}

pub struct LlmGateway {
    config: Option<Config>,
    usage: RefCell<Option<Usage>>,
}

/// One gateway round trip: the answer text/object and what it cost.
pub struct Sent {
    pub output: Value,
    pub model: String,
    pub usage: Usage,
}

impl LlmGateway {
    pub fn disabled() -> LlmGateway {
        LlmGateway { config: None, usage: RefCell::new(None) }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.is_some()
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
        let structured = match get("PULSO_LLM_GATEWAY_STRUCTURED").as_deref().unwrap_or("prompted") {
            "prompted" => Structured::Prompted,
            "native" => Structured::Native,
            "text" => Structured::Text,
            o => return Err(format!("PULSO_LLM_GATEWAY_STRUCTURED {o:?} is not prompted|native|text")),
        };
        let model = get("PULSO_LLM_GATEWAY_MODEL").filter(|m| !m.is_empty()).unwrap_or_else(|| DEFAULT_MODEL.into());
        let (pin, pout) = default_price(&model);
        Ok(LlmGateway {
            config: Some(Config {
                addr,
                key: need("PULSO_LLM_GATEWAY_KEY")?,
                model,
                alias: get("PULSO_LLM_GATEWAY_ALIAS").filter(|m| !m.is_empty()).unwrap_or_else(|| "openrouter".into()),
                max_tokens: num("PULSO_LLM_GATEWAY_MAX_TOKENS", 4000)?,
                timeout_s: num("PULSO_LLM_GATEWAY_TIMEOUT_S", 60)?,
                price_in: dec("PULSO_LLM_GATEWAY_PRICE_IN", pin)?,
                price_out: dec("PULSO_LLM_GATEWAY_PRICE_OUT", pout)?,
                structured,
            }),
            usage: RefCell::new(None),
        })
    }
}

impl LlmGateway {
    pub fn structured(&self) -> Option<Structured> {
        self.config.as_ref().map(|c| c.structured)
    }

    /// One `POST /v1/generate`. In `text` mode `output` is the raw answer string; otherwise the parsed object. The usage of the call
    /// (also of a call whose answer the gateway rejected: the provider charged for it) is kept for `last_usage`.
    pub fn send(&self, req: &ModelRequest, mode: Structured) -> Result<Sent, ModelError> {
        let Some(c) = &self.config else {
            return Err(ModelError::Refused("llm_gateway_disabled: set PULSO_LLM_GATEWAY=enabled with PULSO_LLM_GATEWAY_ADDR and PULSO_LLM_GATEWAY_KEY".into()));
        };
        *self.usage.borrow_mut() = None;
        guard(req)?;
        let wire_mode = if mode == Structured::Native { "native" } else { "prompted" };
        let mut body = json!({
            "prompt": req.system,
            "inputs": req.payload,
            "profile": {"endpoint_alias": c.alias, "model": c.model, "temperature": 0, "max_tokens": c.max_tokens, "timeout_s": c.timeout_s,
                        "structured": wire_mode, "price": {"input_per_mtok": c.price_in, "output_per_mtok": c.price_out}},
            "labels": {"agent": format!("pulso-{}", req.role.as_str())},
        });
        if mode != Structured::Text
            && let Some(schema) = req.payload.get("output_schema").filter(|s| s.is_object())
        {
            body["schema"] = schema.clone();
        }
        let mut headers = vec![("Authorization", format!("Bearer {}", c.key)), ("Content-Type", "application/json".to_string())];
        // story correlation: traceparent (parent = the stage span of the caller) and baggage; headers only, the path never changes
        headers.extend(core_client::trace::headers());
        let t0 = Instant::now();
        let r = request(&c.addr, "POST", "/v1/generate", &headers, Some(body.to_string().as_bytes()), Duration::from_secs(c.timeout_s + 10)).map_err(|e| match e {
            HttpError::Connect(m) => ModelError::Unavailable(format!("gateway_unreachable: {m}")),
            HttpError::Io(m) | HttpError::Protocol(m) => ModelError::Unavailable(format!("gateway_io: {m}")),
        })?;
        let latency_ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
        let doc: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        let usage_of = |d: &Value| -> Option<Usage> {
            Some(Usage { tokens_in: d["tokens_in"].as_u64()?, tokens_out: d["tokens_out"].as_u64()?, cost_usd: d["cost_usd"].as_str().unwrap_or("0").to_string(), latency_ms })
        };
        let kind = doc.pointer("/error/kind").and_then(Value::as_str).unwrap_or("no_error_kind").to_string();
        let usage = if r.status == 200 { usage_of(&doc) } else { usage_of(&doc["error"]) };
        *self.usage.borrow_mut() = usage.clone().or(Some(Usage { latency_ms, ..Usage::default() }));
        match r.status {
            200 => {}
            // the gateway names the rule (a path and a rule, never a value), e.g. `/opportunity: propiedad no permitida "x"`: keep it, it is the feedback
            502 if kind == "invalid_output" => {
                let why: String = doc.pointer("/error/message").and_then(Value::as_str).unwrap_or("").chars().take(160).collect();
                return Err(ModelError::Invalid(format!("gateway_invalid_output: the model answer is not JSON, violates the schema or was truncated ({why})")));
            }
            s => return Err(ModelError::Unavailable(format!("gateway_http_{s}: {kind}"))),
        }
        let output = doc.get("output").cloned().ok_or_else(|| ModelError::Invalid("gateway answered without output".into()))?;
        let model = doc["model"].as_str().filter(|m| !m.is_empty()).unwrap_or(&c.model).to_string();
        Ok(Sent { output, model, usage: usage.unwrap_or(Usage { latency_ms, ..Usage::default() }) })
    }
}

impl ModelPort for LlmGateway {
    fn label(&self) -> Label {
        Label::Gateway
    }
    fn model_id(&self) -> String {
        self.config.as_ref().map_or_else(|| "llm-gateway-disabled".into(), |c| c.model.clone())
    }
    fn last_usage(&self) -> Option<Usage> {
        self.usage.borrow().clone()
    }
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        let mode = self.config.as_ref().map_or(Structured::Prompted, |c| c.structured);
        let sent = self.send(req, mode)?;
        let content = match (&sent.output, mode) {
            (Value::String(t), Structured::Text) => extract_json_object(t).map_err(|e| ModelError::Invalid(format!("answer_not_json:{}", e.code())))?,
            (o, _) if o.is_object() => o.clone(),
            _ => return Err(ModelError::Invalid("gateway output is not a JSON object".into())),
        };
        Ok(ModelAnswer { content, model_id: sent.model, label: Label::Gateway })
    }
}
