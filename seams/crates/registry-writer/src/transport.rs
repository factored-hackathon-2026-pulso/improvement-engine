//! The wire. `Transport` is the seam the offline tests script; `HttpTransport` is plain HTTP/1.1 over `core_client::http`.
use core_client::authorizer::Jws;
use core_client::http::{self, HttpError};
use serde_json::Value;
use std::time::Duration;

pub struct Request<'a> {
    pub method: &'a str,
    pub path: String,
    pub bearer: &'a Jws,
    pub idempotency_key: Option<&'a str>,
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    /// `Null` when the body is not JSON.
    pub body: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// Failed before any byte was written.
    NotSent(String),
    /// The request may have been received.
    Unknown(String),
}

pub trait Transport {
    fn send(&self, req: &Request) -> Result<Reply, TransportError>;
}

pub struct HttpTransport {
    addr: String,
    timeout: Duration,
}

impl HttpTransport {
    /// `addr` is `host:port` (plain HTTP: the local stack; a shared Core behind a TLS edge is not reachable by this).
    pub fn new(addr: &str, timeout: Duration) -> HttpTransport {
        HttpTransport { addr: addr.into(), timeout }
    }
}

impl Transport for HttpTransport {
    fn send(&self, req: &Request) -> Result<Reply, TransportError> {
        let mut headers = vec![("Authorization", format!("Bearer {}", req.bearer.reveal()))];
        if let Some(k) = req.idempotency_key {
            headers.push(("Idempotency-Key", k.to_string()));
        }
        headers.extend(core_client::trace::headers());
        let bytes = req.body.as_ref().map(|b| serde_json::to_vec(b).expect("json"));
        match http::request(&self.addr, req.method, &req.path, &headers, bytes.as_deref(), self.timeout) {
            Ok(r) => Ok(Reply { status: r.status, body: serde_json::from_slice(&r.body).unwrap_or(Value::Null) }),
            Err(HttpError::Connect(m)) => Err(TransportError::NotSent(m)),
            Err(HttpError::Io(m) | HttpError::Protocol(m)) => Err(TransportError::Unknown(m)),
        }
    }
}
