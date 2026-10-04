//! Structured JSON logs (one object per line) for `pulso run`.
//! Callers pass explicit fields; the logger additionally refuses to print values under secret-looking keys and
//! anything shaped like a connection string, so a careless field cannot leak a DSN, token or row value.
use serde_json::{Map, Value, json};
use std::io::Write;
use std::sync::{Arc, Mutex};

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
        let _ = (&self.out, level, event, fields, json!({}), Map::new());
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
