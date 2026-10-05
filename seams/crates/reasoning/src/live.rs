//! Ports for a real run: the REAL llm-gateway (`engine::models::llm_gateway::LlmGateway`) configured from the environment.
//! Scout and Builder use `PULSO_LLM_GATEWAY_MODEL`; the Verifier is a separate port instance and uses
//! `PULSO_LLM_GATEWAY_VERIFIER_MODEL` when set (another model = stronger independence), else the same model with its own prompt
//! and context. The key is read from `PULSO_LLM_GATEWAY_KEY` and is never printed, recorded or part of any report.
use crate::pipeline::Ports;
use engine::models::llm_gateway::LlmGateway;
use std::rc::Rc;

pub fn ports_from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<Ports, String> {
    let main = LlmGateway::from_env(get)?;
    if !main.is_enabled() {
        return Err("the llm gateway is not enabled: set PULSO_LLM_GATEWAY=enabled, PULSO_LLM_GATEWAY_ADDR and PULSO_LLM_GATEWAY_KEY".into());
    }
    let verifier_model = get("PULSO_LLM_GATEWAY_VERIFIER_MODEL").filter(|m| !m.trim().is_empty());
    let verifier = match verifier_model {
        Some(m) => LlmGateway::from_env(&|k: &str| if k == "PULSO_LLM_GATEWAY_MODEL" { Some(m.clone()) } else { get(k) })?,
        None => LlmGateway::from_env(get)?,
    };
    let builder = LlmGateway::from_env(get)?;
    Ok(Ports { scout: Rc::new(main), verifier: Rc::new(verifier), builder: Rc::new(builder) })
}
