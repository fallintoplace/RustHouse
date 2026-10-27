//! Bounded, loopback-only HTTP client for read-only RustHouse peers.
//! This is not a general HTTP or authenticated network transport.

use crate::cluster::Peer;
use crate::{MAX_HEADER_BYTES, MAX_QUERY_BYTES, MAX_RESPONSE_BYTES};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(750);
const IO_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, PartialEq, Eq)]
pub struct PeerResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

pub fn ping(peer: Peer) -> Result<PeerResponse, String> {
    request(peer, "GET", "/ping", b"")
}

pub fn capabilities(peer: Peer) -> Result<PeerResponse, String> {
    request(peer, "GET", "/capabilities", b"")
}

/// Fetch a named table's schema without accepting arbitrary HTTP paths.
pub fn schema(peer: Peer, table: &str) -> Result<PeerResponse, String> {
    let mut bytes = table.bytes();
    if !bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("unsupported peer table identifier".into());
    }
    request(peer, "GET", &format!("/schema?table={table}"), b"")
}

pub fn query(peer: Peer, sql: &str) -> Result<PeerResponse, String> {
    if sql.len() > MAX_QUERY_BYTES || sql.is_empty() {
        return Err("invalid SQL statement length".into());
    }
    request(peer, "POST", "/", sql.as_bytes())
}

/// Never follows redirects, negotiates compression, or connects to non-loopback addresses.
fn request(peer: Peer, method: &str, path: &str, body: &[u8]) -> Result<PeerResponse, String> {
    let address = SocketAddr::V4(peer.address());
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
        .map_err(|error| format!("cannot reach peer: {error}"))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| error.to_string())?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    stream.write_all(body).map_err(|error| error.to_string())?;
    // Include one extra byte so oversized replies cannot look like complete responses.
    let max_wire = MAX_HEADER_BYTES + MAX_RESPONSE_BYTES + 1;
    let mut raw = Vec::new();
    stream
        .take(max_wire as u64)
        .read_to_end(&mut raw)
        .map_err(|error| format!("cannot read peer response: {error}"))?;
    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> Result<PeerResponse, String> {
    let head_end = raw
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .ok_or("peer response has no header terminator")?;
    if head_end + 4 > MAX_HEADER_BYTES {
        return Err("peer headers exceed 8 KiB".into());
    }
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| "peer headers are not UTF-8")?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or("missing peer status")?;
    let mut words = status_line.splitn(3, ' ');
    let version = words.next().ok_or("invalid peer HTTP status")?;
    let code = words.next().ok_or("invalid peer HTTP status")?;
    let reason = words.next().ok_or("invalid peer HTTP status")?;
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0")
        || code.len() != 3
        || !code.bytes().all(|byte| byte.is_ascii_digit())
        || reason.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err("invalid peer HTTP status".into());
    }
    let status = code
        .parse::<u16>()
        .map_err(|_| "invalid peer HTTP status")?;
    if !(200..=599).contains(&status) {
        return Err("unsupported peer HTTP status".into());
    }
    let mut length = None;
    let mut content_type = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("invalid peer header")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() && byte != b'\t')
        {
            return Err("invalid peer header".into());
        }
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() || !value.bytes().all(|b| b.is_ascii_digit()) || value.is_empty() {
                return Err("invalid peer Content-Length".into());
            }
            length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| "invalid peer Content-Length")?,
            );
        } else if name.eq_ignore_ascii_case("content-type") {
            if content_type.is_some() {
                return Err("duplicate peer Content-Type".into());
            }
            content_type = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("content-encoding")
        {
            return Err("unsupported peer transfer encoding".into());
        }
    }
    let length = length.ok_or("missing peer Content-Length")?;
    if length > MAX_RESPONSE_BYTES || raw.len() - head_end - 4 != length {
        return Err("peer response size mismatch or exceeds 8 MiB".into());
    }
    Ok(PeerResponse {
        status,
        content_type: content_type.ok_or("missing peer Content-Type")?,
        body: raw[head_end + 4..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    #[test]
    fn queries_a_loopback_peer_with_exact_sql_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut head = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            assert!(head.starts_with("POST / HTTP/1.1\r\n"));
            assert!(head.contains("Content-Length: 8\r\n"));
            let mut sql = [0u8; 8];
            reader.read_exact(&mut sql).unwrap();
            assert_eq!(&sql, b"SELECT 1");
            drop(reader);
            stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Type: text/tab-separated-values\r\nConnection: close\r\n\r\n1\n1\n"
            ).unwrap();
        });
        let peer = Peer::parse(&format!("127.0.0.1:{port}")).unwrap();
        let result = query(peer, "SELECT 1").unwrap();
        assert_eq!(result.status, 200);
        assert_eq!(result.body, b"1\n1\n");
        server.join().unwrap();
    }

    #[test]
    fn rejects_ambiguous_and_truncated_responses() {
        for reply in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 2\r\nContent-Type: text/plain\r\n\r\nab".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Type: text/plain\r\n\r\nab",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\nContent-Type: text/plain\r\n\r\nab",
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Type: text/plain\r\n\r\nab",
        ] {
            assert!(parse_response(reply).is_err());
        }
    }

    #[test]
    fn rejects_control_bytes_in_peer_http_status_and_headers() {
        let malformed = [
            b"HTTP/1.1 200 OK\nInjected\r\nContent-Length: 2\r\nContent-Type: text/plain\r\n\r\nOk".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: text/plain\nInjected: yes\r\n\r\nOk",
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type : text/plain\r\n\r\nOk",
        ];
        for response in malformed {
            assert!(parse_response(response).is_err());
        }
    }

    #[test]
    fn validates_sql_before_connecting() {
        let peer = Peer::parse("127.0.0.1:9").unwrap();
        assert!(query(peer, "").is_err());
        assert!(query(peer, &"x".repeat(MAX_QUERY_BYTES + 1)).is_err());
    }
}
