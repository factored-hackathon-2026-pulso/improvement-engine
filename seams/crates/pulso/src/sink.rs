//! `HttpSink`: a `RunEventSink` that appends through the debug-api admin route of a RUNNING server (loopback, std only).
//! This is how `pulso demo` writes into the store that `pulso serve` (or `debug-api`) shows in the console.
use debug_api::{NewEvent, RunEventSink};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub struct HttpSink {
    addr: String,
    token: String,
}

impl HttpSink {
    pub fn new(addr: &str, token: &str) -> HttpSink {
        HttpSink { addr: addr.to_string(), token: token.to_string() }
    }
}

impl RunEventSink for HttpSink {
    fn emit(&self, run_id: &str, ev: NewEvent) -> Result<Value, String> {
        let mut body = json!({"kind": ev.kind, "entity_kind": ev.entity_kind, "entity_id": ev.entity_id, "data": ev.data});
        if let Some(t) = ev.occurred_at {
            body["occurred_at"] = json!(t);
        }
        let body = body.to_string();
        let sock = self.addr.to_socket_addrs().map_err(|e| format!("{}: {e}", self.addr))?.next().ok_or_else(|| format!("{}: no address", self.addr))?;
        let mut s = TcpStream::connect_timeout(&sock, Duration::from_secs(3)).map_err(|e| format!("cannot reach debug-api at {}: {e}", self.addr))?;
        s.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
        let req = format!(
            "POST /__admin/v1/runs/{run_id}/events HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.addr,
            self.token,
            body.len()
        );
        s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut text = String::new();
        s.read_to_string(&mut text).map_err(|e| e.to_string())?;
        let (head, payload) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| format!("bad response from debug-api: {head:?}"))?;
        if status != 200 {
            return Err(format!("debug-api refused the append with status {status}"));
        }
        serde_json::from_str(payload).map_err(|e| format!("debug-api answered non-JSON: {e}"))
    }
}
