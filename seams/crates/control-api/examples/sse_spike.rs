//! E9s spike: tiny_http SSE, disconnect detection time and memory per open stream. Run:
//! `cargo run --release --example sse_spike -- [heartbeat_ms] [streams]` (prints two numbers + the raw samples).
use control_api::sse::Sse;
use std::io::Read;
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[repr(C)]
struct Pmc {
    cb: u32,
    page_faults: u32,
    peak_ws: usize,
    ws: usize,
    q1: usize,
    q2: usize,
    q3: usize,
    q4: usize,
    pagefile: usize,
    peak_pagefile: usize,
}
unsafe extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32GetProcessMemoryInfo(p: isize, c: *mut Pmc, cb: u32) -> i32;
}
fn working_set() -> usize {
    let mut c = Pmc { cb: std::mem::size_of::<Pmc>() as u32, page_faults: 0, peak_ws: 0, ws: 0, q1: 0, q2: 0, q3: 0, q4: 0, pagefile: 0, peak_pagefile: 0 };
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    c.ws
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let hb = Duration::from_millis(a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1000));
    let streams: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(500);
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = server.server_addr().to_ip().unwrap();
    let (tx, rx) = mpsc::channel::<(usize, Instant)>();
    thread::spawn(move || {
        let mut n = 0usize;
        for req in server.incoming_requests() {
            n += 1;
            let id = n;
            let tx = tx.clone();
            // one OS thread per stream, small stack: this is the tiny_http cost model
            thread::Builder::new().stack_size(128 * 1024).spawn(move || {
                let mut s = match Sse::start(req) { Ok(s) => s, Err(_) => { let _ = tx.send((id, Instant::now())); return; } };
                let _ = s.event(Some("1"), "hello", "{}");
                loop {
                    thread::sleep(hb);
                    if s.heartbeat().is_err() { let _ = tx.send((id, Instant::now())); return; }
                }
            }).unwrap();
        }
    });
    let open = |addr| -> TcpStream {
        let mut c = TcpStream::connect(addr).unwrap();
        use std::io::Write;
        c.write_all(b"GET /events HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n\r\n").unwrap();
        let mut buf = [0u8; 512];
        let mut got = Vec::new();
        while !String::from_utf8_lossy(&got).contains("event: hello") { let n = c.read(&mut buf).unwrap(); got.extend_from_slice(&buf[..n]); }
        c
    };
    // 1. disconnect detection (clean FIN close), 6 samples; ids start at 1
    let mut det = Vec::new();
    for i in 1..=6 {
        let c = open(addr);
        thread::sleep(Duration::from_millis(137 * i as u64 % hb.as_millis() as u64)); // vary phase against the heartbeat
        let t0 = Instant::now();
        drop(c);
        let (_, t1) = rx.recv_timeout(Duration::from_secs(30)).expect("disconnect never detected");
        det.push(t1.duration_since(t0).as_secs_f64());
    }
    // 2. memory per open stream
    let base = working_set();
    let mut conns: Vec<TcpStream> = Vec::new();
    for _ in 0..streams { conns.push(open(addr)); }
    let loaded = working_set();
    let per_kib = (loaded.saturating_sub(base) as f64 / streams as f64) / 1024.0;
    println!("heartbeat_ms={} streams={streams}", hb.as_millis());
    println!("DISCONNECT_DETECTION_S max={:.3} samples={:?}", det.iter().cloned().fold(0.0, f64::max), det.iter().map(|d| (d * 1000.0).round() / 1000.0).collect::<Vec<_>>());
    println!("MEMORY base_mib={:.1} loaded_mib={:.1} per_stream_kib={:.1} (server threads + client sockets in one process)", base as f64 / 1048576.0, loaded as f64 / 1048576.0, per_kib);
    drop(conns);
}
