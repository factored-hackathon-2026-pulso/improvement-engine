//! tiny_http glue (pattern of control-api): one thread per request, body read capped, everything else delegated to
//! `App::handle`. Open feeds hold a thread each and are bounded separately. Loopback only.
use crate::app::{App, Req, Resp, StreamPlan};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const MAX_BODY: u64 = 1024 * 1024 + 16 * 1024;
const MAX_INFLIGHT: usize = 256;
const MAX_STREAMS: usize = 128;
const STREAM_POLL: Duration = Duration::from_millis(50);

/// Binds `addr`, refusing anything that is not a loopback address (the debug surface has no real authentication).
pub fn bind_loopback(addr: &str) -> Result<tiny_http::Server, String> {
    let a: SocketAddr = addr.parse().map_err(|e| format!("bad address {addr:?}: {e}"))?;
    if !a.ip().is_loopback() {
        return Err(format!("{addr} is not a loopback address; debug-api only binds loopback"));
    }
    tiny_http::Server::http(a).map_err(|e| format!("bind {addr}: {e}"))
}

struct Guard<'a>(&'a AtomicUsize);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn refuse(request: tiny_http::Request, status: u16, code: &str) {
    let _ = request.respond(tiny_http::Response::from_string(format!("{{\"code\":\"{code}\"}}")).with_status_code(status));
}

fn respond(request: tiny_http::Request, resp: Resp) {
    let mut r = tiny_http::Response::from_data(resp.body).with_status_code(resp.status);
    for (k, v) in resp.headers {
        if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            r = r.with_header(h);
        }
    }
    let _ = request.respond(r);
}

static STREAMS: AtomicUsize = AtomicUsize::new(0);

pub fn serve(server: tiny_http::Server, app: Arc<App>) {
    let inflight = Arc::new(AtomicUsize::new(0));
    for mut request in server.incoming_requests() {
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
            let headers: HashMap<String, String> = request.headers().iter().map(|h| (h.field.as_str().as_str().to_ascii_lowercase(), h.value.to_string())).collect();
            let (path, query) = request.url().split_once('?').map_or((request.url(), ""), |(p, q)| (p, q));
            let req = Req { method: request.method().as_str().to_string(), path: path.to_string(), query: query.to_string(), headers, body };
            match app.stream_route(&req) {
                Some(Ok(plan)) => stream(request, &app, &plan),
                Some(Err(resp)) => respond(request, resp),
                None => respond(request, app.handle(&req)),
            }
        });
    }
}

/// SSE: `: open`, the retained events after the cursor (`id: <sequence>`), then live appends; `: hb` comment frames when
/// quiet. A failed write (peer gone) ends the feed, bounded by the heartbeat.
fn stream(request: tiny_http::Request, app: &App, plan: &StreamPlan) {
    if STREAMS.fetch_add(1, Ordering::SeqCst) >= MAX_STREAMS {
        STREAMS.fetch_sub(1, Ordering::SeqCst);
        return refuse(request, 503, "overloaded");
    }
    let _guard = Guard(&STREAMS);
    let mut w = request.into_writer();
    let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n: open\n\n";
    if w.write_all(head).and_then(|()| w.flush()).is_err() {
        return;
    }
    let (mut last, mut quiet) = (plan.after, Instant::now());
    let run = |w: &mut Box<dyn Write + Send>, last: &mut i64, quiet: &mut Instant| -> io::Result<()> {
        loop {
            let batch = app.store().events_after(&plan.run, *last, 200);
            if batch.is_empty() {
                return Ok(());
            }
            for e in batch {
                let seq = e["sequence"].as_i64().unwrap_or(*last);
                if seq != *last + 1 {
                    // a purge overtook this feed: end it so the client reconnects and gets 410, never a silent gap
                    return Err(io::Error::new(io::ErrorKind::Other, "purged past cursor"));
                }
                w.write_all(format!("id: {seq}\ndata: {e}\n\n").as_bytes())?;
                (*last, *quiet) = (seq, Instant::now());
            }
            w.flush()?;
        }
    };
    loop {
        if run(&mut w, &mut last, &mut quiet).is_err() {
            return;
        }
        if quiet.elapsed() >= app.heartbeat() {
            if w.write_all(b": hb\n\n").and_then(|()| w.flush()).is_err() {
                return;
            }
            quiet = Instant::now();
        }
        thread::sleep(STREAM_POLL);
    }
}
