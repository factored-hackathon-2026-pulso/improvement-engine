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

/// Wraps a transport so that EVERY request carries a credential minted by the engine's service identity: the bearer the caller
/// passes is a placeholder and is replaced. The credential is short-lived; it is refreshed before expiry (`ServiceIdentity::credential`)
/// and, if the Core still answers 401 (skewed clock, rotated key), minted again once and the request is repeated (a 401 means nothing was done).
pub struct MintingTransport {
    inner: std::sync::Arc<dyn Transport + Send + Sync>,
    identity: std::sync::Arc<core_client::service_identity::ServiceIdentity>,
}

impl MintingTransport {
    pub fn new(inner: std::sync::Arc<dyn Transport + Send + Sync>, identity: std::sync::Arc<core_client::service_identity::ServiceIdentity>) -> MintingTransport {
        MintingTransport { inner, identity }
    }

    fn once(&self, req: &Request, bearer: &Jws) -> Result<Reply, TransportError> {
        self.inner.send(&Request { method: req.method, path: req.path.clone(), bearer, idempotency_key: req.idempotency_key, body: req.body.clone() })
    }
}

impl Transport for MintingTransport {
    fn send(&self, req: &Request) -> Result<Reply, TransportError> {
        let first = self.once(req, &self.identity.credential())?;
        if first.status != 401 {
            return Ok(first);
        }
        self.once(req, &self.identity.renew())
    }
}

#[cfg(test)]
mod minting_tests {
    use super::*;
    use core_client::service_identity::{Clock, ServiceIdentity};
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    /// Records the bearer of every request; answers from a script.
    struct Spy {
        seen: Mutex<Vec<String>>,
        statuses: Mutex<Vec<u16>>,
    }
    impl Transport for Spy {
        fn send(&self, req: &Request) -> Result<Reply, TransportError> {
            self.seen.lock().unwrap().push(req.bearer.reveal().to_string());
            let s = { let mut q = self.statuses.lock().unwrap(); if q.is_empty() { 200 } else { q.remove(0) } };
            Ok(Reply { status: s, body: Value::Null })
        }
    }

    fn rig(statuses: Vec<u16>) -> (MintingTransport, Arc<Spy>, Arc<AtomicI64>) {
        let t = Arc::new(AtomicI64::new(1_800_000_000));
        let t2 = t.clone();
        let clock: Clock = Arc::new(move || t2.load(Ordering::SeqCst));
        let spy = Arc::new(Spy { seen: Mutex::new(vec![]), statuses: Mutex::new(statuses) });
        let id = Arc::new(ServiceIdentity::new("k1", [9u8; 32]).with_clock(clock));
        (MintingTransport::new(spy.clone(), id), spy, t)
    }

    fn req<'a>(b: &'a Jws) -> Request<'a> {
        Request { method: "GET", path: "/v1/registry/proposals".into(), bearer: b, idempotency_key: Some("k"), body: Some(serde_json::json!({"a": 1})) }
    }

    #[test]
    fn the_placeholder_bearer_is_replaced_by_a_minted_credential() {
        let (m, spy, _) = rig(vec![]);
        let placeholder = Jws::new("placeholder-not-a-credential".into());
        assert_eq!(m.send(&req(&placeholder)).unwrap().status, 200);
        let seen = spy.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_ne!(seen[0], "placeholder-not-a-credential");
        assert_eq!(seen[0].split('.').count(), 3);
    }

    #[test]
    fn a_long_running_job_gets_a_new_credential_before_the_old_one_expires() {
        let (m, spy, t) = rig(vec![]);
        let p = Jws::new(String::new());
        m.send(&req(&p)).unwrap();
        t.fetch_add(120, Ordering::SeqCst);
        m.send(&req(&p)).unwrap();
        t.fetch_add(200, Ordering::SeqCst); // 320 s after the first mint: the first one is dead, the second is in its refresh window
        m.send(&req(&p)).unwrap();
        let seen = spy.seen.lock().unwrap();
        assert_eq!(seen[0], seen[1], "inside the lifetime the credential is reused");
        assert_ne!(seen[1], seen[2], "after the refresh window a new one is minted");
    }

    /// A transport whose `send` takes `secs` of (fake) wall time, like an evaluate that runs for minutes.
    struct Slow {
        inner: Spy,
        clock: Arc<AtomicI64>,
        secs: i64,
    }
    impl Transport for Slow {
        fn send(&self, req: &Request) -> Result<Reply, TransportError> {
            let r = self.inner.send(req);
            self.clock.fetch_add(self.secs, Ordering::SeqCst);
            r
        }
    }

    #[test]
    fn an_evaluate_that_outlives_the_credential_ttl_is_not_resent_and_the_next_request_gets_a_new_credential() {
        // the Core checks the credential when the request arrives (minted just before, 300 s TTL); the 600 s call must not be
        // retried or re-signed mid-flight, and the following request must not reuse the long-dead credential
        let clock = Arc::new(AtomicI64::new(1_800_000_000));
        let c2 = clock.clone();
        let clk: Clock = Arc::new(move || c2.load(Ordering::SeqCst));
        let slow = Arc::new(Slow { inner: Spy { seen: Mutex::new(vec![]), statuses: Mutex::new(vec![]) }, clock: clock.clone(), secs: 600 });
        let m = MintingTransport::new(slow.clone(), Arc::new(ServiceIdentity::new("k1", [9u8; 32]).with_clock(clk)));
        let p = Jws::new(String::new());
        assert_eq!(m.send(&req(&p)).unwrap().status, 200);
        assert_eq!(m.send(&req(&p)).unwrap().status, 200);
        let seen = slow.inner.seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "one send per call, no resend");
        assert_ne!(seen[0], seen[1], "a credential older than its TTL is never reused");
    }

    #[test]
    fn a_401_is_retried_once_with_a_renewed_credential() {
        // e.g. the Core has not yet re-read a rotated staff-keys file: the same request is repeated once, still minted, never the placeholder
        let (m, spy, t) = rig(vec![401, 200]);
        let r = m.send(&req(&Jws::new(String::new()))).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(spy.seen.lock().unwrap().len(), 2);
        assert!(spy.seen.lock().unwrap().iter().all(|b| b.split('.').count() == 3));
        spy.statuses.lock().unwrap().push(401);
        t.fetch_add(3, Ordering::SeqCst);
        m.send(&req(&Jws::new(String::new()))).unwrap();
        let seen = spy.seen.lock().unwrap();
        assert_ne!(seen[2], seen[3], "a renewal after time passed is a new credential");
    }

    #[test]
    fn a_second_401_is_returned_not_looped() {
        let (m, spy, _) = rig(vec![401, 401, 200]);
        assert_eq!(m.send(&req(&Jws::new(String::new()))).unwrap().status, 401);
        assert_eq!(spy.seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn other_statuses_are_not_retried() {
        let (m, spy, _) = rig(vec![403]);
        assert_eq!(m.send(&req(&Jws::new(String::new()))).unwrap().status, 403);
        assert_eq!(spy.seen.lock().unwrap().len(), 1);
    }
}
