use control_api::{app::{App, Config}, auth::KeyRing, server, store::MemStore};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

fn start() -> String {
    let ring = Arc::new(KeyRing::from_json(&serde_json::json!({})).unwrap());
    let app = Arc::new(App::new(Config { ring, upload_pin: None, admin: false }, Box::new(MemStore::default())));
    let s = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let addr = s.server_addr().to_ip().unwrap().to_string();
    std::thread::spawn(move || server::serve(s, app));
    addr
}

fn raw(addr: &str, req: &[u8]) -> String {
    let mut c = TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    let _ = c.write_all(req);
    let mut out = String::new();
    let _ = c.read_to_string(&mut out);
    out
}

#[test]
fn oversized_content_length_is_413_without_the_body_being_sent() {
    let addr = start();
    // announce 1 GiB, send none: the server must answer from the header alone instead of waiting for the body
    let out = raw(&addr, b"POST /internal/v1/broker/artifacts HTTP/1.1\r\nHost: x\r\nContent-Length: 1073741824\r\nConnection: close\r\n\r\n");
    assert!(out.starts_with("HTTP/1.1 413"), "{out}");
}

#[test]
fn admin_channel_is_404_when_disabled() {
    let addr = start();
    let out = raw(&addr, b"POST /_e2e/config HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
    assert!(out.starts_with("HTTP/1.1 404"), "{out}");
}

#[test]
fn malformed_json_does_not_kill_the_server() {
    let addr = start();
    for _ in 0..3 {
        let out = raw(&addr, b"POST /internal/v1/core-task-bindings HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\nAuthorization: Bearer a.b.c\r\nConnection: close\r\n\r\n{{{");
        assert!(out.starts_with("HTTP/1.1 401"), "{out}");
    }
}
