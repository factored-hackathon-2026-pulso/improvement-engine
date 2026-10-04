//! Server-Sent Events over a `tiny_http` request. We take the raw connection writer (`Request::into_writer`) and write
//! the head ourselves (`Connection: close`, no chunking): a write error is the disconnect signal, so a periodic
//! heartbeat comment bounds detection time (a closed peer RSTs the first write, the second write errors).
use std::io::{self, Write};

pub struct Sse {
    w: Box<dyn Write + Send>,
}

impl Sse {
    pub fn start(request: tiny_http::Request) -> io::Result<Sse> {
        let mut w = request.into_writer();
        w.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n")?;
        w.flush()?;
        Ok(Sse { w })
    }

    pub fn event(&mut self, id: Option<&str>, event: &str, data: &str) -> io::Result<()> {
        let mut s = String::new();
        if let Some(id) = id {
            s.push_str(&format!("id: {id}\n"));
        }
        s.push_str(&format!("event: {event}\n"));
        for line in data.split('\n') {
            s.push_str(&format!("data: {line}\n"));
        }
        s.push('\n');
        self.w.write_all(s.as_bytes())?;
        self.w.flush()
    }

    pub fn heartbeat(&mut self) -> io::Result<()> {
        self.w.write_all(b": hb\n\n")?;
        self.w.flush()
    }
}
