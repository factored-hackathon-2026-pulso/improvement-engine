//! `pulso healthcheck`: container HEALTHCHECK for images without curl/wget.
use crate::config::normalize_base_path;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// GET http://127.0.0.1:<port><base_path>/readyz; Ok only on HTTP 200. Errors never carry response bodies or secrets.
pub fn check(port: u16, base_path: &str, timeout: Duration) -> Result<(), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("cannot reach readyz: {:?}", e.kind()))?;
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(timeout));
    write!(s, "GET {base_path}/readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").map_err(|e| format!("cannot send readyz request: {:?}", e.kind()))?;
    let mut head = Vec::new();
    let _ = s.take(512).read_to_end(&mut head);
    let head = String::from_utf8_lossy(&head);
    let status = head.split(' ').nth(1).and_then(|c| c.parse::<u16>().ok()).ok_or("readyz sent no HTTP status")?;
    if status == 200 { Ok(()) } else { Err(format!("readyz returned {status}")) }
}

/// Entry for `pulso healthcheck [--port N]`: port from the flag, else PULSO_LISTEN_ADDR, else 8080;
/// prefix from PULSO_BASE_PATH. Exit 0 healthy, 1 unhealthy, 2 usage. Prints one short line on failure only.
pub fn main(args: &[String]) -> i32 {
    let mut port: Option<u16> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match (a.as_str(), it.next().map(|v| v.parse::<u16>())) {
            ("--port", Some(Ok(p))) => port = Some(p),
            _ => {
                eprintln!("pulso healthcheck: usage: pulso healthcheck [--port N]");
                return 2;
            }
        }
    }
    let port = port.unwrap_or_else(|| std::env::var("PULSO_LISTEN_ADDR").ok().and_then(|a| a.parse::<SocketAddr>().ok()).map_or(8080, |a| a.port()));
    let base = std::env::var("PULSO_BASE_PATH").ok().and_then(|b| normalize_base_path(&b).ok()).unwrap_or_default();
    match check(port, &base, Duration::from_secs(2)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("pulso healthcheck: {e}");
            1
        }
    }
}
