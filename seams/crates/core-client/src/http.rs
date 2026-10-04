//! Minimal blocking HTTP/1.1 over `std::net::TcpStream` (plain TCP; the bridge is local). One request per
//! connection (`Connection: close`).
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Debug)]
pub struct RawResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub enum HttpError {
    /// Failed before any byte was written: the request was NOT sent.
    Connect(String),
    /// Failed after the request may have reached the server: outcome unknown.
    Io(String),
    Protocol(String),
}

pub fn request(
    addr: &str,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<RawResponse, HttpError> {
    let sock = addr
        .to_socket_addrs()
        .map_err(|e| HttpError::Connect(e.to_string()))?
        .next()
        .ok_or_else(|| HttpError::Connect("no address".into()))?;
    let mut s = TcpStream::connect_timeout(&sock, timeout).map_err(|e| HttpError::Connect(e.to_string()))?;
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(timeout));
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAccept: application/json\r\n");
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        head.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).map_err(|e| HttpError::Io(e.to_string()))?;
    if let Some(b) = body {
        s.write_all(b).map_err(|e| HttpError::Io(e.to_string()))?;
    }
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| HttpError::Io(e.to_string()))?;
    parse(&raw)
}

fn parse(raw: &[u8]) -> Result<RawResponse, HttpError> {
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| HttpError::Io("truncated response".into()))?;
    let head = String::from_utf8_lossy(&raw[..end]).to_string();
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| HttpError::Protocol("bad status line".into()))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())))
        .collect();
    let rest = &raw[end + 4..];
    let get = |n: &str| headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    let body = if get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        dechunk(rest)?
    } else if let Some(len) = get("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if rest.len() < len {
            return Err(HttpError::Io("truncated body".into()));
        }
        rest[..len].to_vec()
    } else {
        rest.to_vec()
    };
    Ok(RawResponse { status, headers, body })
}

fn dechunk(mut d: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut out = Vec::new();
    loop {
        let nl = d
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| HttpError::Io("bad chunk".into()))?;
        let size_txt = String::from_utf8_lossy(&d[..nl]).to_string();
        let size = usize::from_str_radix(size_txt.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| HttpError::Protocol("bad chunk size".into()))?;
        d = &d[nl + 2..];
        if size == 0 {
            return Ok(out);
        }
        if d.len() < size + 2 {
            return Err(HttpError::Io("truncated chunk".into()));
        }
        out.extend_from_slice(&d[..size]);
        d = &d[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chunked_and_content_length() {
        let c = parse(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n").unwrap();
        assert_eq!((c.status, c.body.as_slice()), (200, &b"abcde"[..]));
        let l = parse(b"HTTP/1.1 409 X\r\nContent-Length: 2\r\n\r\n{}").unwrap();
        assert_eq!((l.status, l.body.as_slice()), (409, &b"{}"[..]));
        assert!(matches!(parse(b"HTTP/1.1 200 X\r\nContent-Length: 9\r\n\r\n{}"), Err(HttpError::Io(_))));
    }
}
