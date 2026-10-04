//! TcpRelay: a fault-injecting TCP relay in front of a real server (the Rust twin of the e2e-core live
//! fault-injection relay used for `evaluation_result_lost`). The request is forwarded and EXECUTED upstream; the
//! response of the first `drop_responses` connections is swallowed and the client socket closed.
#![allow(dead_code)]
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

pub struct TcpRelay {
    pub addr: String,
    stop: Arc<AtomicBool>,
    pub dropped: Arc<AtomicU32>,
    handle: Option<JoinHandle<()>>,
}

impl TcpRelay {
    pub fn start(upstream: &str, drop_responses: u32) -> TcpRelay {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU32::new(0));
        let (s2, d2, up) = (stop.clone(), dropped.clone(), upstream.to_string());
        let handle = std::thread::spawn(move || {
            for c in l.incoming() {
                if s2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut client) = c else { continue };
                let mut req = Vec::new();
                let mut tmp = [0u8; 4096];
                // read one HTTP request (head + content-length body)
                let ok = loop {
                    if let Some(p) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&req[..p]).to_ascii_lowercase();
                        let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                        if req.len() >= p + 4 + len {
                            break true;
                        }
                    }
                    match client.read(&mut tmp) {
                        Ok(0) | Err(_) => break false,
                        Ok(n) => req.extend_from_slice(&tmp[..n]),
                    }
                };
                if !ok {
                    continue;
                }
                let Ok(mut u) = TcpStream::connect(&up) else { continue };
                let _ = u.write_all(&req);
                let mut resp = Vec::new();
                let _ = u.read_to_end(&mut resp);
                if d2.load(Ordering::SeqCst) < drop_responses {
                    d2.fetch_add(1, Ordering::SeqCst);
                    continue; // response lost: close the client socket without a byte
                }
                let _ = client.write_all(&resp);
            }
        });
        TcpRelay { addr, stop, dropped, handle: Some(handle) }
    }
}

impl Drop for TcpRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
