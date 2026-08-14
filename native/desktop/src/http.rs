//! A one-purpose HTTP client: GET, over loopback, nothing else.
//!
//! Everything's HTTP server is the only thing VOLT talks to, it is always on 127.0.0.1, and it
//! always answers with `Content-Length` and `Connection: Close`. That is a small enough target
//! that pulling in a real HTTP stack (and, with it, a TLS stack we would never use) costs more
//! than it saves. Forty lines of `TcpStream` covers it.
//!
//! The narrowness is also the security story. There is no way to point this at a remote host: it
//! resolves nothing, follows no redirects, and speaks no TLS, so a malicious search result cannot
//! turn it into a way out of the machine.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::time::Duration;

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// GET `path` (already percent-encoded) from 127.0.0.1:`port`.
///
/// `path` must start with `/` and is written into the request line verbatim — callers are
/// responsible for encoding, which is also why they must not let a raw newline through. We reject
/// control characters here rather than trusting that, because a smuggled `\r\n` would let a search
/// string append headers of its own.
pub fn get(port: u16, path: &str, timeout: Duration) -> Result<Response, String> {
    if !path.starts_with('/') || path.bytes().any(|b| b < 0x20) {
        return Err("bad request path".into());
    }

    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let mut sock = TcpStream::connect_timeout(&addr.into(), timeout)
        .map_err(|e| format!("cannot reach Everything's HTTP server on port {port}: {e}"))?;
    sock.set_read_timeout(Some(timeout)).ok();
    sock.set_write_timeout(Some(timeout)).ok();

    // HTTP/1.0 so the server closes the connection when it is done and read-to-end terminates.
    // That also sidesteps chunked encoding, which 1.1 would let it choose.
    let req = format!(
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nUser-Agent: VOLT\r\nAccept: */*\r\n\r\n"
    );
    sock.write_all(req.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;

    let mut raw = Vec::new();
    sock.read_to_end(&mut raw)
        .map_err(|e| format!("read failed: {e}"))?;

    // Split on the blank line. `windows(4)` rather than a string search because the body is
    // arbitrary bytes — a WAV, usually — and must never go through UTF-8 validation.
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("malformed response: no header terminator")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let body = raw[split + 4..].to_vec();

    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .ok_or("malformed response: no status line")?;

    let content_type = lines
        .find(|l| l.to_ascii_lowercase().starts_with("content-type:"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default();

    Ok(Response {
        status,
        content_type,
        body,
    })
}
