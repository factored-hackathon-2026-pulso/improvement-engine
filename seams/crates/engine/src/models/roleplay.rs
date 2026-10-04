//! `Roleplay`: replay-only client of the roleplay-llm queue protocol (`roleplay-queue/1`). It reads
//! `<queue>/responses/<key>.json` (the key is the shim's replay key) and NEVER writes a request: producing answers is the
//! shim's and the responder's job. Every answer is labelled `roleplay`, never `real`.
//!
//! Known gap: the Python key replaces uuids and `binding|job|artifact` ids by first-seen ordinals; this port does not
//! reimplement that substitution and refuses (`replay_key_unsupported_ids`) any payload or system prompt that would need it.
use super::{Label, ModelAnswer, ModelError, ModelPort, ModelRequest, guard};
use core_client::canon::sha256_hex;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const PROTOCOL: &str = "roleplay-queue/1";
pub const PROVENANCE: &str = "agent_roleplay";
const VOLATILE: [&str; 4] = ["run_id", "turn_id", "session_id", "labels"];
const FORBIDDEN_KEYS: [&str; 4] = ["quality", "score", "confidence", "rating"];
const RESPONSE_KEYS: [&str; 6] = ["protocol", "key", "provenance", "quality_claims", "responder", "content"];

pub struct Roleplay {
    pub queue: PathBuf,
}

impl Roleplay {
    pub fn new(queue: &Path) -> Roleplay {
        Roleplay { queue: queue.to_path_buf() }
    }
}

fn word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// True when the Python shim would substitute an ordinal in `s` (uuid or `binding|job|artifact` id at a word boundary).
fn needs_ordinals(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len()).filter(|&i| i == 0 || !word(b[i - 1])).any(|i| {
        let rest = &b[i..];
        let uuid = rest.len() >= 36
            && [8usize, 4, 4, 4, 12].iter().zip([0usize, 9, 14, 19, 24]).all(|(n, at)| rest[at..at + n].iter().all(u8::is_ascii_hexdigit))
            && [8usize, 13, 18, 23].iter().all(|&at| rest[at] == b'-')
            && rest.get(36).is_none_or(|c| !word(*c));
        let prefixed = ["binding", "job", "artifact"].iter().any(|p| {
            rest.starts_with(p.as_bytes())
                && rest.get(p.len()).is_some_and(|c| matches!(c, b'-' | b'_' | b':'))
                && rest.get(p.len() + 1).is_some_and(u8::is_ascii_alphanumeric)
                && rest.len() >= p.len() + 2 + 5
                && rest[p.len() + 2..p.len() + 7].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
        });
        uuid || prefixed
    })
}

fn any_string(v: &Value, f: &dyn Fn(&str) -> bool) -> bool {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter().any(|x| any_string(x, f)),
        Value::Object(m) => m.values().any(|x| any_string(x, f)),
        _ => false,
    }
}

/// Canonical JSON as the shim writes it: sorted keys, compact separators, raw unicode.
fn canon(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canon(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canon(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

fn normalise(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), if VOLATILE.contains(&k.as_str()) { Value::String("<volatile>".into()) } else { normalise(x) })).collect()),
        Value::Array(a) => Value::Array(a.iter().map(normalise).collect()),
        other => other.clone(),
    }
}

/// The shim's `replay_key(system, inputs)`: sha256 over `{"inputs": normalised, "stage": sha256(system)[:16]}`.
pub fn replay_key(system: &str, payload: &Value) -> Result<String, String> {
    if needs_ordinals(system) || any_string(payload, &needs_ordinals) {
        return Err("replay_key_unsupported_ids".into());
    }
    let stage = &sha256_hex(system.as_bytes())[..16];
    let doc = serde_json::json!({"stage": stage, "inputs": normalise(payload)});
    let mut s = String::new();
    canon(&doc, &mut s);
    Ok(sha256_hex(s.as_bytes())[..32].to_string())
}

fn forbidden_key(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, x)| FORBIDDEN_KEYS.contains(&k.as_str()) || forbidden_key(x)),
        Value::Array(a) => a.iter().any(forbidden_key),
        _ => false,
    }
}

impl ModelPort for Roleplay {
    fn label(&self) -> Label {
        Label::Roleplay
    }
    fn model_id(&self) -> String {
        "agent_roleplay".into()
    }
    fn call(&self, req: &ModelRequest) -> Result<ModelAnswer, ModelError> {
        guard(req)?;
        let key = replay_key(&req.system, &req.payload).map_err(ModelError::Refused)?;
        let path = self.queue.join("responses").join(format!("{key}.json"));
        let text = std::fs::read_to_string(&path).map_err(|_| ModelError::Unavailable(format!("replay_miss:{key}")))?;
        let bad = |w: &str| ModelError::Invalid(format!("roleplay response {key}: {w}"));
        let doc: Value = serde_json::from_str(&text).map_err(|_| bad("not JSON"))?;
        let o = doc.as_object().ok_or_else(|| bad("not an object"))?;
        if o.len() != RESPONSE_KEYS.len() || !RESPONSE_KEYS.iter().all(|k| o.contains_key(*k)) {
            return Err(bad("fields are not exactly protocol,key,provenance,quality_claims,responder,content"));
        }
        if o["protocol"] != PROTOCOL || o["key"] != key.as_str() || o["provenance"] != PROVENANCE || o["quality_claims"] != "forbidden" {
            return Err(bad("protocol, key, provenance or quality_claims label is wrong"));
        }
        let resp = o["responder"].as_object().ok_or_else(|| bad("responder is not an object"))?;
        let (id, role) = (resp.get("id").and_then(Value::as_str), resp.get("role").and_then(Value::as_str));
        match (id, role) {
            (Some(id), Some(role)) if !id.is_empty() && role == req.role.as_str() => {
                let c = o["content"].as_object().ok_or_else(|| bad("content is not an object"))?;
                if c.get("kind").and_then(Value::as_str) != Some("final") {
                    return Err(bad("content.kind is not final"));
                }
                let output = c.get("output").filter(|v| v.is_object()).ok_or_else(|| bad("content.output missing"))?;
                if forbidden_key(output) {
                    return Err(bad("output carries a quality claim"));
                }
                Ok(ModelAnswer { content: output.clone(), model_id: format!("{PROVENANCE}:{id}"), label: Label::Roleplay })
            }
            _ => Err(bad("responder id missing or responder role is not the requested role")),
        }
    }
}
