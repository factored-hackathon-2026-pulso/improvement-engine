//! Ports for a real run: the REAL llm-gateway (`engine::models::llm_gateway::LlmGateway`) configured from the environment.
//! Scout and Builder use `PULSO_LLM_GATEWAY_MODEL`; the Verifier is a separate port instance and uses
//! `PULSO_LLM_GATEWAY_VERIFIER_MODEL` when set (another model = stronger independence), else `DEFAULT_VERIFIER_MODEL` (`xiaomi/mimo-v2.6-pro`: same vendor family as the generator, other tier; labelled `same_family_other_tier`). The key is read from `PULSO_LLM_GATEWAY_KEY` and is never printed, recorded or part of any report.
use crate::pipeline::Ports;
use engine::models::llm_gateway::LlmGateway;
use std::rc::Rc;

/// Verifier tier when `PULSO_LLM_GATEWAY_VERIFIER_MODEL` is unset: the larger sibling of the generation model (same vendor family, other tier).
pub const DEFAULT_VERIFIER_MODEL: &str = "xiaomi/mimo-v2.6-pro";

pub fn ports_from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<Ports, String> {
    // Default answer packaging for the roles is `text` (BLD1: the gateway returns the raw text and the engine extracts ONE JSON object
    // and validates it strictly; with `prompted` the gateway discarded 8 of 60 answers it could not parse or that carried a stray key,
    // and with `text` the same model/prompts lost none to packaging). `PULSO_LLM_GATEWAY_STRUCTURED` overrides.
    let get = &|k: &str| match k {
        "PULSO_LLM_GATEWAY_STRUCTURED" => get(k).or_else(|| Some("text".to_string())),
        _ => get(k),
    };
    let main = LlmGateway::from_env(get)?;
    if !main.is_enabled() {
        return Err("the llm gateway is not enabled: set PULSO_LLM_GATEWAY=enabled, PULSO_LLM_GATEWAY_ADDR and PULSO_LLM_GATEWAY_KEY".into());
    }
    let verifier_model = Some(get("PULSO_LLM_GATEWAY_VERIFIER_MODEL").filter(|m| !m.trim().is_empty()).unwrap_or_else(|| DEFAULT_VERIFIER_MODEL.to_string()));
    let verifier = match verifier_model {
        Some(m) => LlmGateway::from_env(&|k: &str| if k == "PULSO_LLM_GATEWAY_MODEL" { Some(m.clone()) } else { get(k) })?,
        None => LlmGateway::from_env(get)?,
    };
    // The Builder tier: `PULSO_LLM_GATEWAY_BUILDER_MODEL` (default: the generation model, flash). With
    // `PULSO_LLM_GATEWAY_BUILDER_ESCALATION_MODEL` (e.g. `xiaomi/mimo-v2.6-pro`) a finding the primary tier could not compile after its
    // bounded retries is tried once more on that stronger tier (user policy: pro for tasks that need more reasoning).
    let with_model = |m: String| LlmGateway::from_env(&|k: &str| if k == "PULSO_LLM_GATEWAY_MODEL" { Some(m.clone()) } else { get(k) });
    let builder = match get("PULSO_LLM_GATEWAY_BUILDER_MODEL").filter(|m| !m.trim().is_empty()) {
        Some(m) => with_model(m)?,
        None => LlmGateway::from_env(get)?,
    };
    let escalation = match get("PULSO_LLM_GATEWAY_BUILDER_ESCALATION_MODEL").filter(|m| !m.trim().is_empty()) {
        Some(m) => Some(Rc::new(with_model(m)?) as Rc<dyn engine::models::ModelPort>),
        None => None,
    };
    let mut ports = Ports::new(Rc::new(main), Rc::new(verifier), Rc::new(builder));
    ports.builder_escalation = escalation;
    Ok(ports)
}
