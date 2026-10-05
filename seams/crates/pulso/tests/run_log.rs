//! Structured JSON log lines: shape, one line per event, and no secret / DSN / row value can reach the output.
use pulso::run::log::Logger;
use serde_json::{Value, json};
use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);
impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Buf {
    fn lines(&self) -> Vec<String> {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap().lines().map(String::from).collect()
    }
}

#[test]
fn each_event_is_one_json_line_with_ts_level_event_and_fields() {
    let buf = Buf::default();
    let log = Logger::new(Box::new(buf.clone()));
    log.info("run_started", json!({"data_mode": "dataset", "tasks": 2}));
    log.warn("retry", json!({"note": "line one\nline two"}));
    let lines = buf.lines();
    assert_eq!(lines.len(), 2, "embedded newlines must not split a record: {lines:?}");
    let a: Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(a["level"], "info");
    assert_eq!(a["event"], "run_started");
    assert_eq!(a["data_mode"], "dataset");
    assert_eq!(a["tasks"], 2);
    assert!(a["ts_ms"].as_u64().unwrap() > 1_700_000_000_000);
    let b: Value = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(b["level"], "warn");
}

#[test]
fn secret_keys_and_connection_strings_are_redacted() {
    let buf = Buf::default();
    let log = Logger::new(Box::new(buf.clone()));
    log.error(
        "boom",
        json!({
            "database_url": "postgres://svc:pw123@host/db",
            "debug_token": "tok-abcdef",
            "Password": "x",
            "detail": "connect failed postgresql://u:pw456@h:5432/d refused",
            "row": {"customer": "Ana Perez"},
            "values": ["a", "b"],
            "attempt": 3
        }),
    );
    let line = buf.lines().remove(0);
    for leak in ["pw123", "tok-abcdef", "pw456", "Ana Perez", "svc:"] {
        assert!(!line.contains(leak), "{leak} leaked: {line}");
    }
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["database_url"], "<redacted>");
    assert_eq!(v["attempt"], 3);
    assert!(v["detail"].as_str().unwrap().contains("<redacted>"));
}

#[test]
fn reserved_keys_cannot_be_overwritten_by_fields() {
    let buf = Buf::default();
    let log = Logger::new(Box::new(buf.clone()));
    log.info("real", json!({"event": "forged", "level": "debug", "ts_ms": 1}));
    let v: Value = serde_json::from_str(&buf.lines()[0]).unwrap();
    assert_eq!(v["event"], "real");
    assert_eq!(v["level"], "info");
    assert!(v["ts_ms"].as_u64().unwrap() > 1_700_000_000_000);
}
