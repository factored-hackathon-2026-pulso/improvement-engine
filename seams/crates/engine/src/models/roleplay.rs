//! `Roleplay`: replay-only client of the roleplay-llm queue protocol (`roleplay-queue/1`).
use super::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct Roleplay {
    pub queue: PathBuf,
}

impl Roleplay {
    pub fn new(queue: &Path) -> Roleplay {
        Roleplay { queue: queue.to_path_buf() }
    }
}

pub fn replay_key(_system: &str, _payload: &Value) -> Result<String, String> {
    Err("todo".into())
}

impl ModelPort for Roleplay {
    fn label(&self) -> Label {
        Label::Roleplay
    }
    fn model_id(&self) -> String {
        "agent_roleplay".into()
    }
    fn call(&self, _req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        Err(ModelError::Unavailable("todo".into()))
    }
}
