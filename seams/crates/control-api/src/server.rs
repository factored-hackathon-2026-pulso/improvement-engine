//! tiny_http glue: one thread per request, body read capped, everything else delegated to `App::handle`.
use crate::app::{App, Req};
use crate::debug::StreamPlan;
use crate::sse::Sse;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

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
            let _guard = Guard(&inflight);
            let mut body = Vec::new();
            let _ = request.as_reader().take(MAX_BODY + 1).read_to_end(&mut body);
            let headers = request.headers().iter().map(|h| (h.field.as_str().as_str().to_ascii_lowercase(), h.value.to_string())).collect();
            let (path, query) = request.url().split_once('?').map_or((request.url(), ""), |(p, q)| (p, q));
            let req = Req { method: request.method().as_str().to_string(), path: path.to_string(), query: query.to_string(), headers, body };
            if let Some(plan) = app.stream_route(&req) {
                match plan {
                    Ok(plan) => stream(request, &app, &plan),
                    Err(resp) => respond(request, resp),
                }
                return;
            }
            let resp = app.handle(&req);
            respond(request, resp);
        });
    }
}

struct Guard<'a>(&'a AtomicUsize);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn respond(request: tiny_http::Request, resp: crate::app::Resp) {
    let ct = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let _ = request.respond(tiny_http::Response::from_data(resp.body).with_status_code(resp.status).with_header(ct));
}

/// Open feeds hold a thread each (see SPIKE.md); bounded separately from ordinary requests.
const MAX_STREAMS: usize = 128;
/// A feed ends after this long: the JWT that opened it is short-lived, so the client re-authenticates on resume.
const STREAM_MAX_AGE: Duration = Duration::from_secs(300);
const STREAM_HEARTBEAT: Duration = Duration::from_secs(1);
const STREAM_POLL: Duration = Duration::from_millis(100);
static STREAMS: AtomicUsize = AtomicUsize::new(0);

/// Writes the run-event feed until the peer goes away (a failed write, bounded by the heartbeat) or the age cap.
fn stream(request: tiny_http::Request, app: &App, plan: &StreamPlan) {
    if STREAMS.fetch_add(1, Ordering::SeqCst) >= MAX_STREAMS {
        STREAMS.fetch_sub(1, Ordering::SeqCst);
        return refuse(request, 503, "overloaded");
    }
    let _guard = Guard(&STREAMS);
    let Ok(mut sse) = Sse::start(request) else { return };
    let (started, mut quiet, mut last) = (Instant::now(), Instant::now(), plan.after);
    while started.elapsed() < STREAM_MAX_AGE {
        for e in app.run_events_after(&plan.tenant, &plan.run, last) {
            let seq = e["sequence"].as_i64().unwrap_or(last);
            if sse.event(Some(&seq.to_string()), "run_event", &e.to_string()).is_err() {
                return;
            }
            (last, quiet) = (seq, Instant::now());
        }
        if quiet.elapsed() >= STREAM_HEARTBEAT {
            if sse.heartbeat().is_err() {
                return;
            }
            quiet = Instant::now();
        }
        thread::sleep(STREAM_POLL);
    }
}
