//! Structured JSON logs (one object per line) for `pulso run`.
//! Callers pass explicit fields; the logger additionally refuses to print values under secret-looking keys and
//! anything shaped like a connection string, so a careless field cannot leak a DSN, token or row value.
use serde_json::{Map, Value, json};
use std::io::Write;
use std::sync::{Arc, Mutex};

const REDACTED: &str = "<redacted>";
const SENSITIVE: &[&str] = &["password", "passwd", "secret", "token", "dsn", "url", "credential", "authorization", "api_key", "apikey", "row", "rows", "value", "values", "payload", "body"];

fn sensitive_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    SENSITIVE.iter().any(|s| k.contains(s))
}

/// Replaces anything shaped like `scheme://...` credentials in free text, recursively.
fn scrub(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(scrub_str(&s)),
        Value::Array(a) => Value::Array(a.into_iter().map(scrub).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| if sensitive_key(&k) { (k, json!(REDACTED)) } else { (k, scrub(v)) }).collect()),
        other => other,
    }
}

fn scrub_str(s: &str) -> String {
    s.split(' ')
        .map(|w| if w.contains("://") { REDACTED } else { w })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone)]
pub struct Logger {
    out: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Logger {
    pub fn new(out: Box<dyn Write + Send>) -> Logger {
        Logger { out: Arc::new(Mutex::new(out)) }
    }
    pub fn stdout() -> Logger {
        Logger::new(Box::new(std::io::stdout()))
    }
    pub fn log(&self, level: &str, event: &str, fields: Value) {
        let mut rec = Map::new();
        if let Value::Object(m) = fields {
            for (k, v) in m {
                if !matches!(k.as_str(), "ts_ms" | "level" | "event") {
                    rec.insert(k.clone(), if sensitive_key(&k) { json!(REDACTED) } else { scrub(v) });
                }
            }
        }
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        rec.insert("ts_ms".into(), json!(ts));
        rec.insert("level".into(), json!(level));
        rec.insert("event".into(), json!(event));
        let line = Value::Object(rec).to_string();
        if let Ok(mut out) = self.out.lock() {
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
    }
    pub fn info(&self, event: &str, fields: Value) {
        self.log("info", event, fields)
    }
    pub fn warn(&self, event: &str, fields: Value) {
        self.log("warn", event, fields)
    }
    pub fn error(&self, event: &str, fields: Value) {
        self.log("error", event, fields)
    }
}
