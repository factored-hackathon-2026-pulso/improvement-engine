//! tiny_http glue: one thread per request, body read capped, everything else delegated to `App::handle`.
use crate::app::{App, Req};
use std::io::Read;
use std::sync::Arc;
use std::thread;

const MAX_BODY: u64 = 1024 * 1024 + 16 * 1024;

pub fn serve(server: tiny_http::Server, app: Arc<App>) {
    for mut request in server.incoming_requests() {
        let app = app.clone();
        thread::spawn(move || {
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
