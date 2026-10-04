//! Test handlers (not the steps crate: those take file/runner inputs, not linear payloads).
use abi::*;

pub struct Append(pub &'static str);

impl JobHandler for Append {
    fn id(&self) -> HandlerId {
        HandlerId(format!("append-{}", self.0))
    }
    fn run(&self, _f: &Fence, i: &InputEnvelope) -> Result<OutputEnvelope, HandlerError> {
        Ok(OutputEnvelope {
            payload: format!("{}{}", i.payload, self.0),
            events: vec![format!("step{}:{}:{}", i.step_index, self.0, i.payload)],
            effect: EffectState::NoEffect,
        })
    }
}

pub fn handlers() -> Vec<Box<dyn JobHandler>> {
    vec![Box::new(Append("a")), Box::new(Append("b")), Box::new(Append("c"))]
}
