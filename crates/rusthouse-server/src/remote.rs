//! Bounded HTTP client for explicitly configured RustHouse peers.
//! Non-loopback transport is opt-in, plaintext, and unauthenticated.

mod response_parse;

use self::response_parse::*;

use crate::cluster::Peer;
use crate::{MAX_HEADER_BYTES, MAX_QUERY_BYTES, MAX_RESPONSE_BYTES};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

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

/// A deadline on the entire response, not on each individual read.
/// A peer that trickles one byte per read must not keep a coordinator thread alive.
fn read_with_deadline(stream: &mut TcpStream, timeout: Duration) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    let max_wire = MAX_HEADER_BYTES + MAX_RESPONSE_BYTES + 1;
    let mut raw = Vec::new();
    // The response header is immutable after its terminator. Parse it once,
    // then compare subsequent network reads against the cached wire length.
    let mut expected_length = None;
    let mut buffer = [0u8; 8192];
    loop {
        let remaining = timeout
            .checked_sub(started.elapsed())
            .filter(|duration| !duration.is_zero())
            .ok_or("peer response deadline exceeded")?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|error| error.to_string())?;
        let available = max_wire
            .checked_sub(raw.len())
            .ok_or("peer response exceeds wire limit")?;
        if available == 0 {
            return Err("peer response exceeds wire limit".into());
        }
        let capacity = available.min(buffer.len());
        let size = stream.read(&mut buffer[..capacity]).map_err(|error| {
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                "peer response deadline exceeded".to_owned()
            } else {
                format!("cannot read peer response: {error}")
            }
        })?;
        if size == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..size]);
        if expected_length.is_none() {
            expected_length = expected_wire_length(&raw)?;
        }
        // Content-Length is authoritative for this restricted peer protocol.
        // Never wait for EOF after receiving the entire declared body.
        if expected_length.is_some_and(|expected| raw.len() >= expected) {
            break;
        }
    }
    Ok(raw)
}

/// Never follows redirects, negotiates compression, or resolves hostnames.
fn request(peer: Peer, method: &str, path: &str, body: &[u8]) -> Result<PeerResponse, String> {
    let address = SocketAddr::V4(peer.address());
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
        .map_err(|error| format!("cannot reach peer: {error}"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| error.to_string())?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        peer.address().ip(),
        body.len()
    )
    .map_err(|error| error.to_string())?;
    stream.write_all(body).map_err(|error| error.to_string())?;
    let raw = read_with_deadline(&mut stream, IO_TIMEOUT)?;
    parse_response_owned(raw)
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
    fn complete_content_length_does_not_wait_for_peer_disconnect() {
        use std::sync::mpsc;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (release, keep_open) = mpsc::channel::<()>();
        let server =
            std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: text/plain\r\n\r\nOK"
            ).unwrap();
                // The client must finish before the server closes the connection.
                keep_open.recv_timeout(Duration::from_secs(3)).unwrap();
            });
        let mut stream = TcpStream::connect(address).unwrap();
        let bytes = read_with_deadline(&mut stream, Duration::from_secs(2)).unwrap();
        assert_eq!(parse_response(&bytes).unwrap().body, b"OK");
        release.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn completed_response_still_rejects_invalid_size_and_duplicate_headers() {
        let missing = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Type: text/plain\r\n\r\nOK";
        assert_eq!(
            expected_wire_length(missing).unwrap(),
            Some(missing.len() + 1)
        );
        assert!(parse_response(missing).is_err());
        let duplicate = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\nContent-Type: text/plain\r\n\r\nOK";
        assert!(expected_wire_length(duplicate).is_err());
    }

    #[test]
    fn peer_trickling_bytes_cannot_extend_total_read_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            for _ in 0..8 {
                if stream.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        });
        let mut stream = TcpStream::connect(address).unwrap();
        let begun = Instant::now();
        let error = read_with_deadline(&mut stream, Duration::from_millis(115)).unwrap_err();
        assert!(
            error.contains("deadline") || error.contains("timed out"),
            "{error}"
        );
        assert!(begun.elapsed() < Duration::from_millis(280));
        drop(stream);
        writer.join().unwrap();
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
    fn rejects_empty_content_type_from_peers() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: \t\r\n\r\nOk";
        assert_eq!(
            parse_response(response).unwrap_err(),
            "empty peer Content-Type"
        );
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
