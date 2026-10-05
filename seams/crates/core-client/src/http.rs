//! Minimal blocking HTTP/1.1 over `std::net::TcpStream` (plain TCP; the bridge is local). One request per
//! connection (`Connection: close`).
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Hard cap on a response (head + body); the bridge caps results at 256 KiB.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

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
    // Request splitting: no CR, LF, NUL or other control byte may reach the head through a header name or value, the method or the path.
    let line_safe = |t: &str| !t.bytes().any(|b| (b < 0x20 && b != b'\t') || b == 0x7f);
    if !line_safe(method) || !line_safe(path) || path.contains(' ') || method.contains(' ') {
        return Err(HttpError::Connect("refused: control byte or space in the request line".into()));
    }
    if let Some((k, _)) = headers.iter().find(|(k, v)| !line_safe(k) || !line_safe(v) || k.is_empty() || k.contains(':') || k.contains(' ')) {
        return Err(HttpError::Connect(format!("refused: header {k:?} carries a control byte (CR/LF) or is malformed")));
    }
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
    // Bounded read: one byte past the cap is enough to refuse.
    (&mut s)
        .take(MAX_RESPONSE_BYTES as u64 + 1024 * 16 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| HttpError::Io(e.to_string()))?;
    parse(&raw)
}

fn parse(raw: &[u8]) -> Result<RawResponse, HttpError> {
    if raw.len() > MAX_RESPONSE_BYTES + 16 * 1024 {
        return Err(HttpError::Protocol("response too large".into()));
    }
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
    if rest.len() > MAX_RESPONSE_BYTES {
        return Err(HttpError::Protocol("response too large".into()));
    }
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
        if size > MAX_RESPONSE_BYTES || d.len() < size + 2 {
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

    #[test]
    fn hostile_chunk_size_is_an_error_not_a_panic() {
        let r = parse(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\nabc\r\n0\r\n\r\n");
        assert!(matches!(r, Err(HttpError::Protocol(_)) | Err(HttpError::Io(_))));
    }

    #[test]
    fn a_header_or_request_line_with_cr_lf_is_refused_before_anything_is_sent() {
        // no listener exists at this address: the refusal must come first, as `Connect` (not sent), with the exact reason
        let t = Duration::from_millis(200);
        let crlf = "\r\n";
        for h in [("traceparent", format!("00-aa{crlf}Authorization: Bearer evil")), ("baggage", "a\nb".to_string()), ("X-A\r\nB", "v".to_string()), ("X-A:", "v".to_string())] {
            match request("127.0.0.1:9", "POST", "/v1/generate", &[h.clone()], None, t) {
                Err(HttpError::Connect(m)) => assert!(m.starts_with("refused: header"), "{h:?} {m}"),
                o => panic!("{h:?} {o:?}"),
            }
        }
        for (m, p) in [("POST", format!("/v1/runs HTTP/1.1{crlf}X: y")), ("GET", "/a b".to_string()), ("POST\r\n", "/a".to_string())] {
            assert!(matches!(request("127.0.0.1:9", m, &p, &[], None, t), Err(HttpError::Connect(x)) if x.starts_with("refused: control byte")), "{m:?} {p:?}");
        }
    }

    #[test]
    fn oversized_body_is_refused() {
        let mut raw = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        raw.extend(std::iter::repeat(b'a').take(MAX_RESPONSE_BYTES + 1));
        assert!(matches!(parse(&raw), Err(HttpError::Protocol(_))));
    }
}
