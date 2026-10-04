//! tiny_http glue: one thread per request, body read capped, everything else delegated to `App::handle`.
use crate::app::{App, Req};
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

const MAX_BODY: u64 = 1024 * 1024 + 16 * 1024;

/// Bound on concurrently handled requests (each holds a thread; tiny_http exposes no socket read timeout).
const MAX_INFLIGHT: usize = 256;

fn refuse(request: tiny_http::Request, status: u16, code: &str) {
    let body = format!("{{\"code\":\"{code}\"}}");
    let _ = request.respond(tiny_http::Response::from_string(body).with_status_code(status));
}

pub fn serve(server: tiny_http::Server, app: Arc<App>) {
    let inflight = Arc::new(AtomicUsize::new(0));
    for mut request in server.incoming_requests() {
        // Decide from the header alone: never wait for (or buffer) a body that is declared too large.
        if request.body_length().is_some_and(|n| n as u64 > MAX_BODY) {
            refuse(request, 413, "payload_too_large");
            continue;
        }
        if inflight.fetch_add(1, Ordering::SeqCst) >= MAX_INFLIGHT {
            inflight.fetch_sub(1, Ordering::SeqCst);
            refuse(request, 503, "overloaded");
            continue;
        }
        let (app, inflight) = (app.clone(), inflight.clone());
        thread::spawn(move || {
            let _guard = Guard(inflight);
            let mut body = Vec::new();
            let _ = request.as_reader().take(MAX_BODY + 1).read_to_end(&mut body);
            let headers = request.headers().iter().map(|h| (h.field.as_str().as_str().to_ascii_lowercase(), h.value.to_string())).collect();
            let path = request.url().split('?').next().unwrap_or("").to_string();
            let resp = app.handle(&Req { method: request.method().as_str().to_string(), path, headers, body });
            let ct = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            let out = tiny_http::Response::from_data(resp.body).with_status_code(resp.status).with_header(ct);
            let _ = request.respond(out);
        });
    }
}

struct Guard(Arc<AtomicUsize>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
