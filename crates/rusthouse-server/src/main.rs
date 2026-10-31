use rusthouse_server::cluster::{self, Cluster, Peer};
use rusthouse_server::distributed;
use rusthouse_server::health;
use rusthouse_server::{
    Config, MAX_CONNECTIONS, MAX_HEADER_BYTES, MAX_QUERY_BYTES, Response, handle_request,
};
use std::io::{self, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const MAX_CONFIGURED_CONNECTIONS: usize = 64;
static HTTP_REQUESTS_TOTAL: AtomicU64 = AtomicU64::new(0);
static HTTP_ERRORS_TOTAL: AtomicU64 = AtomicU64::new(0);

fn metrics_response(requests: u64, errors: u64) -> Response {
    Response {
        status: "200 OK",
        content_type: "text/plain; version=0.0.4; charset=utf-8",
        body: format!(
            "# HELP rusthouse_http_requests_total HTTP requests processed by this server.\n# TYPE rusthouse_http_requests_total counter\nrusthouse_http_requests_total {requests}\n# HELP rusthouse_http_errors_total HTTP requests with non-200 results.\n# TYPE rusthouse_http_errors_total counter\nrusthouse_http_errors_total {errors}\n"
        )
        .into_bytes(),
    }
}
/// Release a connection permit even when a worker panics or cannot start.
struct ActiveRequest(Arc<AtomicUsize>);

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}


fn retryable_accept_error(kind: io::ErrorKind) -> bool {
    matches!(kind, io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted)
}

/// Bound concurrent read-only analytical requests to avoid unbounded
/// thread, decoded-block and response-buffer amplification.
fn parse_max_connections(value: &str) -> io::Result<usize> {
    let workers = value.parse::<usize>()
        .map_err(|_| io::Error::other("invalid max connections"))?;
    if !(1..=MAX_CONFIGURED_CONNECTIONS).contains(&workers) {
        return Err(io::Error::other("max connections must be 1..64"));
    }
    Ok(workers)
}

fn usage() -> &'static str {
    "Usage: rusthouse-server [--port <1-65535>] [--part <frozen-part-or-snapshot> | --database <directory>] [--max-rows <n>] [--max-connections <1-64>] [--peer IPv4:<port>]... [--bind <IPv4-or-IPv6-loopback>] [--allow-private-network]\n\
     Listens on 127.0.0.1 by default. Private networking requires opt-in. No TLS or authentication.\n"
}

fn parse_header(line: &str) -> Result<(&str, &str), Response> {
    let (key, value) = line
        .split_once(':')
        .ok_or_else(|| Response::text("400 Bad Request", "invalid HTTP header\n"))?;
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() && byte != b'\t')
    {
        return Err(Response::text("400 Bad Request", "invalid HTTP header\n"));
    }
    Ok((key, value))
}

fn parse_request_line(line: &str) -> Result<(&str, &str), Response> {
    let mut words = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (words.next(), words.next(), words.next(), words.next())
    else {
        return Err(Response::text("400 Bad Request", "invalid HTTP request line\n"));
    };
    if method.is_empty()
        || target.is_empty()
        || version.is_empty()
        || !matches!(version, "HTTP/1.1" | "HTTP/1.0")
        || !target.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(Response::text(
            "400 Bad Request",
            "invalid HTTP request line\n",
        ));
    }
    if method != "GET" && method != "POST" {
        return Err(Response::text(
            "405 Method Not Allowed",
            "only GET and POST\n",
        ));
    }
    Ok((method, target))
}

/// Only numeric bind addresses (and localhost for loopback) are trusted as
/// Host values. This prevents DNS rebinding against an unauthenticated server.
fn host_matches_listener(host: &str, listener: SocketAddr) -> bool {
    if let Some(value) = host.strip_prefix('[') {
        // RFC 3986 requires IPv6 address literals in HTTP Host to be
        // bracketed. Compare addresses, not spellings: compressed and
        // expanded forms refer to the same bound listener.
        let Some((address, suffix)) = value.split_once(']') else {
            return false;
        };
        let port = if suffix.is_empty() {
            None
        } else if let Some(port) = suffix.strip_prefix(':') {
            Some(port)
        } else {
            return false;
        };
        let SocketAddr::V6(bound) = listener else {
            return false;
        };
        return address.parse::<std::net::Ipv6Addr>().ok() == Some(*bound.ip())
            && port.is_none_or(|value| value.parse::<u16>().ok() == Some(bound.port()));
    }
    // Unbracketed IPv6 is not an HTTP authority. A textual IPv4 address or
    // loopback hostname may have a single optional port.
    let (name, port) = match host.split_once(':') {
        Some((name, port)) if !port.contains(':') => (name, Some(port)),
        Some(_) => return false,
        None => (host, None),
    };
    if port.is_some_and(|port| port.parse::<u16>().ok() != Some(listener.port())) {
        return false;
    }
    name == listener.ip().to_string()
        || (listener.ip().is_loopback() && name.eq_ignore_ascii_case("localhost"))
}


const REQUEST_READ_DEADLINE: Duration = Duration::from_secs(5);

fn request_remaining(started: Instant, deadline: Duration) -> Result<Duration, Response> {
    deadline
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| Response::text("408 Request Timeout", "request read deadline exceeded\n"))
}

fn request_read_error(error: io::Error, incomplete: &'static str) -> Response {
    if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        Response::text("408 Request Timeout", "request read deadline exceeded\n")
    } else {
        Response::text("400 Bad Request", incomplete)
    }
}

fn read_body_chunk_with_deadline(
    reader: &mut impl Read,
    buffer: &mut [u8],
    started: Instant,
    deadline: Duration,
) -> Result<usize, Response> {
    loop {
        request_remaining(started, deadline)?;
        match reader.read(buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(request_read_error(error, "incomplete HTTP request body\n"));
            }
            Ok(bytes) => return Ok(bytes),
        }
    }
}

fn read_request_with_deadline(
    stream: &mut TcpStream,
    deadline: Duration,
) -> Result<(String, String, Vec<u8>, bool), Response> {
    let started = Instant::now();
    let mut input = BufReader::new(stream);
    let mut head = Vec::new();
    // Read only until the headers end, without consuming the beginning of the body.
    while !head.ends_with(b"\r\n\r\n") {
        let remaining = request_remaining(started, deadline)?;
        input
            .get_mut()
            .set_read_timeout(Some(remaining))
            .map_err(|error| request_read_error(error, "cannot set request read timeout\n"))?;
        let mut byte = [0u8];
        input
            .read_exact(&mut byte)
            .map_err(|error| request_read_error(error, "incomplete HTTP request headers\n"))?;
        head.push(byte[0]);
        if head.len() > MAX_HEADER_BYTES {
            return Err(Response::text(
                "413 Payload Too Large",
                "HTTP headers too large\n",
            ));
        }
    }
    let header_text = std::str::from_utf8(&head)
        .map_err(|_| Response::text("400 Bad Request", "HTTP headers are not UTF-8\n"))?;
    let mut lines = header_text.split("\r\n");
    let first = lines
        .next()
        .ok_or_else(|| Response::text("400 Bad Request", "missing HTTP request line\n"))?;
    let (method, target) = parse_request_line(first)?;
    let mut size = None;
    let mut transfer_encoding = false;
    let mut accept_lz4 = false;
    let mut seen_accept = false;
    let mut host = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (key, value) = parse_header(line)?;
        if key.eq_ignore_ascii_case("host") {
            if host.replace(value.trim()).is_some() {
                return Err(Response::text("400 Bad Request", "duplicate Host header\n"));
            }
        }
        if key.eq_ignore_ascii_case("expect") {
            return Err(Response::text(
                "417 Expectation Failed",
                "HTTP expectations are unsupported\n",
            ));
        }
        if key.eq_ignore_ascii_case("content-length") {
            if size.is_some() {
                return Err(Response::text(
                    "400 Bad Request",
                    "duplicate Content-Length\n",
                ));
            }
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Response::text(
                    "400 Bad Request",
                    "invalid Content-Length\n",
                ));
            }
            size = Some(value.parse::<usize>().map_err(|_| {
                Response::text("413 Payload Too Large", "request body too large\n")
            })?);
        }
        if key.eq_ignore_ascii_case("transfer-encoding") {
            transfer_encoding = true;
        }
        if key.eq_ignore_ascii_case("accept-encoding") {
            if seen_accept {
                return Err(Response::text("400 Bad Request", "duplicate Accept-Encoding\n"));
            }
            seen_accept = true;
            accept_lz4 = value.trim() == "x-rusthouse-lz4";
        }
    }
    let listener = input
        .get_ref()
        .local_addr()
        .map_err(|_| Response::text("400 Bad Request", "cannot validate Host\n"))?;
    if host.is_some_and(|host| !host_matches_listener(host, listener))
        || (host.is_none() && first.ends_with(" HTTP/1.1"))
    {
        return Err(Response::text(
            "400 Bad Request",
            "Host does not match listening address\n",
        ));
    }
    if transfer_encoding {
        return Err(Response::text(
            "400 Bad Request",
            "Transfer-Encoding is unsupported\n",
        ));
    }
    if method == "POST" && size.is_none() {
        return Err(Response::text(
            "411 Length Required",
            "POST requires Content-Length\n",
        ));
    }
    let size = size.unwrap_or(0);
    if method == "GET" && size != 0 {
        return Err(Response::text(
            "400 Bad Request",
            "GET must not contain a request body\n",
        ));
    }
    if size > MAX_QUERY_BYTES {
        return Err(Response::text(
            "413 Payload Too Large",
            "request body too large\n",
        ));
    }
    let mut body = vec![0u8; size];
    let mut received = 0;
    while received < size {
        let remaining = request_remaining(started, deadline)?;
        input
            .get_mut()
            .set_read_timeout(Some(remaining))
            .map_err(|error| request_read_error(error, "cannot set request read timeout\n"))?;
        let count = read_body_chunk_with_deadline(
            &mut input, &mut body[received..], started, deadline,
        )?;
        if count == 0 {
            return Err(Response::text(
                "400 Bad Request",
                "incomplete HTTP request body\n",
            ));
        }
        received += count;
    }
    Ok((method.to_owned(), target.to_owned(), body, accept_lz4 && method == "POST" && target == "/"))
}

fn read_request(stream: &mut TcpStream) -> Result<(String, String, Vec<u8>, bool), Response> {
    read_request_with_deadline(stream, REQUEST_READ_DEADLINE)
}

/// Enforce one total response-write budget, not a fresh timeout per syscall.
fn write_with_deadline(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let remaining = deadline.checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "response write deadline exceeded"))?;
        stream.set_write_timeout(Some(remaining))?;
        // Limit each syscall to 64 KiB and recalculate the budget after it.
        let written = stream.write(&bytes[..bytes.len().min(64 * 1024)])?;
        if written == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "client closed during response"));
        }
        bytes = &bytes[written..];
    }
    Ok(())
}


fn respond(stream: &mut TcpStream, response: Response, allow_lz4: bool) -> io::Result<()> {
    // Compress only large successful analytical responses and only when the
    // caller opted into our private peer encoding. Older clients are unchanged.
    let compressed = if allow_lz4 && response.status == "200 OK" && response.body.len() >= 4096 {
        let candidate = lz4_flex::compress_prepend_size(&response.body);
        (candidate.len() + 64 < response.body.len()).then_some(candidate)
    } else {
        None
    };
    let (body, encoding) = match compressed.as_ref() {
        Some(bytes) => (bytes.as_slice(), "Content-Encoding: x-rusthouse-lz4\r\n"),
        None => (response.body.as_slice(), ""),
    };
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}Connection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        response.status,
        response.content_type,
        body.len(),
        encoding,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    write_with_deadline(stream, header.as_bytes(), deadline)?;
    write_with_deadline(stream, body, deadline)
}

fn configure_client_timeouts(stream: &TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(())
}

fn serve(mut stream: TcpStream, config: &Config, cluster: &Cluster) {
    if configure_client_timeouts(&stream).is_err() {
        // Never process an unbounded request when socket safeguards fail.
        return;
    }
    let request = read_request(&mut stream);
    let accept_lz4 = request.as_ref().is_ok_and(|(_, _, _, encoded)| *encoded);
    let response = match request {
        Ok((method, target, body, _)) if target == "/metrics" => {
            if method != "GET" || !body.is_empty() {
                Response::text("405 Method Not Allowed", "metrics requires empty GET\n")
            } else {
                metrics_response(
                    HTTP_REQUESTS_TOTAL.load(Ordering::Relaxed),
                    HTTP_ERRORS_TOTAL.load(Ordering::Relaxed),
                )
            }
        }
        Ok((method, target, body, _)) if target == "/cluster" => {
            cluster::handle(&method, &body, cluster)
        }
        Ok((method, target, body, _)) if target == "/cluster/health" => {
            health::handle(&method, &body, cluster)
        }
        Ok((method, target, body, _)) if target == "/distributed" => {
            distributed::handle(&method, &body, cluster, config.max_rows)
        }
        Ok((method, target, body, _)) => handle_request(&method, &target, &body, config),
        Err(response) => response,
    };
    HTTP_REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);
    if response.status != "200 OK" {
        HTTP_ERRORS_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
    let _ = respond(&mut stream, response, accept_lz4);
}

/// Allow IPv6 only on ::1 without extending the experimental
/// unauthenticated peer transport beyond the existing private IPv4 policy.
fn permitted_listener_bind(bind: IpAddr, allow_private_network: bool) -> bool {
    match bind {
        IpAddr::V4(address) => address == Ipv4Addr::LOCALHOST
            || (allow_private_network && cluster::is_private_address(address)),
        IpAddr::V6(address) => address == Ipv6Addr::LOCALHOST,
    }
}

fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut port = 8123u16;
    let mut part = None;
    let mut database = None;
    let mut max_rows = 10_000usize;
    let mut max_connections = MAX_CONNECTIONS;
    let mut peers = Vec::new();
    let mut bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut allow_private_network = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print!("{}", usage());
                return Ok(());
            }
            "--port" => {
                port = args
                    .next()
                    .ok_or_else(|| io::Error::other("missing port"))?
                    .parse::<u16>()
                    .map_err(|_| io::Error::other("invalid port"))?;
                if port == 0 {
                    return Err(io::Error::other("port must be nonzero"));
                }
            }
            "--part" => {
                part = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| io::Error::other("missing part path"))?,
                ));
            }
            "--database" => {
                database = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| io::Error::other("missing database path"))?,
                ));
            }
            "--max-connections" => {
                let value = args.next()
                    .ok_or_else(|| io::Error::other("missing max connections"))?;
                max_connections = parse_max_connections(&value)?;
            }
            "--peer" => {
                peers.push(
                    args.next()
                        .ok_or_else(|| io::Error::other("missing peer"))?,
                );
            }
            "--bind" => {
                bind = args
                    .next()
                    .ok_or_else(|| io::Error::other("missing bind IP address"))?
                    .parse::<IpAddr>()
                    .map_err(|_| io::Error::other("bind address must be numeric IP"))?;
            }
            "--allow-private-network" => allow_private_network = true,
            "--max-rows" => {
                max_rows = args
                    .next()
                    .ok_or_else(|| io::Error::other("missing row cap"))?
                    .parse::<usize>()
                    .map_err(|_| io::Error::other("invalid row cap"))?;
                if max_rows > 250_000 {
                    return Err(io::Error::other("--max-rows exceeds 250000"));
                }
            }
            other => {
                return Err(io::Error::other(format!(
                    "unknown argument: {other}\n{}",
                    usage()
                )));
            }
        }
    }
    if part.is_some() && database.is_some() {
        return Err(io::Error::other("--part and --database cannot be combined"));
    }
    if !permitted_listener_bind(bind, allow_private_network) {
        return Err(io::Error::other(
            "non-loopback bind requires --allow-private-network and private IPv4; IPv6 supports only ::1",
        ));
    }
    let peers: Vec<Peer> = peers
        .iter()
        .map(|address| {
            Peer::parse_with_private_network(address, allow_private_network)
                .map_err(io::Error::other)
        })
        .collect::<io::Result<_>>()?;
    let cluster = Arc::new(Cluster::new(peers).map_err(io::Error::other)?);
    if cluster
        .peers()
        .iter()
        .any(|peer| IpAddr::V4(*peer.address().ip()) == bind && peer.port() == port)
    {
        return Err(io::Error::other("server cannot be its own peer"));
    }
    let config = Arc::new(Config {
        part,
        database,
        max_rows,
    });
    let listener = TcpListener::bind((bind, port))?;
    eprintln!("RustHouse read-only HTTP listening on http://{}", listener.local_addr()?);
    let active = Arc::new(AtomicUsize::new(0));
    for accepted in listener.incoming() {
        let stream = match accepted {
            Ok(stream) => stream,
            Err(error) if retryable_accept_error(error.kind()) => continue,
            Err(error) => return Err(error),
        };
        let mut count = active.load(Ordering::Relaxed);
        let acquired = loop {
            if count >= max_connections {
                break false;
            }
            match active.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break true,
                Err(updated) => count = updated,
            }
        };
        if !acquired {
            let mut stream = stream;
            // Busy replies run on the accept thread, outside serve()'s
            // per-connection timeout setup. Never block acceptance here.
            if stream
                .set_write_timeout(Some(Duration::from_millis(250)))
                .is_err()
            {
                continue;
            }
            let _ = respond(
                &mut stream,
                Response::text("503 Service Unavailable", "server busy\n"),
                false,
            );
            continue;
        }
        let config = Arc::clone(&config);
        let cluster = Arc::clone(&cluster);
        let permit = ActiveRequest(Arc::clone(&active));
        if let Err(error) = std::thread::Builder::new().spawn(move || {
            let _permit = permit;
            serve(stream, &config, &cluster);
        }) {
            // The failed spawn drops its closure and releases the permit.
            eprintln!("RustHouse cannot start query worker: {error}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod ipv6_bind_policy_tests {
    use super::*;

    #[test]
    fn enables_ipv6_loopback_only_without_expanding_private_network_access() {
        for allow_private in [false, true] {
            assert!(permitted_listener_bind("127.0.0.1".parse().unwrap(), allow_private));
            assert!(permitted_listener_bind("::1".parse().unwrap(), allow_private));
            assert!(!permitted_listener_bind("::".parse().unwrap(), allow_private));
            assert!(!permitted_listener_bind("fc00::1".parse().unwrap(), allow_private));
            assert!(!permitted_listener_bind("fe80::1".parse().unwrap(), allow_private));
            assert!(!permitted_listener_bind("8.8.8.8".parse().unwrap(), allow_private));
        }
        assert!(!permitted_listener_bind("10.1.2.3".parse().unwrap(), false));
        assert!(permitted_listener_bind("10.1.2.3".parse().unwrap(), true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_token_header_names_and_optional_tabs() {
        assert_eq!(
            parse_header("Content-Length: 3").ok(),
            Some(("Content-Length", " 3"))
        );
        assert_eq!(
            parse_header("X_Request: hello\tworld").ok(),
            Some(("X_Request", " hello\tworld"))
        );
    }

    #[test]
    fn rejects_invalid_header_names_and_embedded_controls() {
        for header in [
            "NoColon",
            ": value",
            "Content-Length : 3",
            "Bad Name: value",
            "X-Test: hello\u{0000}world",
            "X-Test: hello\u{001b}world",
        ] {
            assert_eq!(
                parse_header(header).unwrap_err().status,
                "400 Bad Request",
                "{header:?}"
            );
        }
    }
}

#[cfg(test)]
mod request_line_spacing_regressions {
    use super::*;

    #[test]
    fn accepts_standard_http_request_lines() {
        assert!(matches!(
            parse_request_line("GET /ping HTTP/1.1"),
            Ok(("GET", "/ping"))
        ));
        assert!(matches!(
            parse_request_line("POST / HTTP/1.0"),
            Ok(("POST", "/"))
        ));
    }

    #[test]
    fn rejects_control_and_non_ascii_target_bytes() {
        for target in ["/\0", "/\t", "/\u{7f}", "/café"] {
            let line = format!("GET {target} HTTP/1.1");
            assert_eq!(
                parse_request_line(&line)
                    .err()
                    .map(|response| response.status),
                Some("400 Bad Request")
            );
        }
        assert!(parse_request_line("GET /?query=SELECT+1 HTTP/1.1").is_ok());
    }

    #[test]
    fn rejects_tabs_repeated_spaces_and_trailing_spaces() {
        for line in [
            "GET\t/ping HTTP/1.1",
            "GET  /ping HTTP/1.1",
            "GET /ping  HTTP/1.1",
            "GET /ping HTTP/1.1 ",
            "GET\u{00a0}/ping HTTP/1.1",
        ] {
            assert_eq!(
                parse_request_line(line)
                    .err()
                    .map(|response| response.status),
                Some("400 Bad Request")
            );
        }
    }
}

#[cfg(test)]
mod accept_error_policy_tests {
    use super::retryable_accept_error;
    use std::io;

    #[test]
    fn only_expected_transient_accept_errors_are_retryable() {
        for kind in [io::ErrorKind::Interrupted, io::ErrorKind::ConnectionAborted] {
            assert!(retryable_accept_error(kind));
        }
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
            assert!(!retryable_accept_error(kind));
        }
    }
}

#[cfg(test)]
mod compressed_peer_response_tests {
    use super::*;
    use std::net::Shutdown;

    #[test]
    fn negotiated_sql_response_compresses_without_changing_tsv_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
            let mut output = Vec::new();
            stream.read_to_end(&mut output).unwrap();
            output
        });
        let (mut stream, _) = listener.accept().unwrap();
        let plain = b"user\tcount\n".repeat(8192);
        respond(&mut stream, Response {
            status: "200 OK", content_type: "text/tab-separated-values; charset=utf-8", body: plain.clone(),
        }, true).unwrap();
        drop(stream);
        let raw = client.join().unwrap();
        let end = raw.windows(4).position(|bytes| bytes == b"\r\n\r\n").unwrap() + 4;
        assert!(raw[..end].windows(b"Content-Encoding: x-rusthouse-lz4".len())
            .any(|bytes| bytes == b"Content-Encoding: x-rusthouse-lz4"));
        assert_eq!(lz4_flex::decompress_size_prepended(&raw[end..]).unwrap(), plain);
    }
}

#[cfg(test)]
mod concurrency_capacity_tests {
    use super::*;

    #[test]
    fn bounds_parallel_query_workers_before_binding_server() {
        assert_eq!(parse_max_connections("1").unwrap(), 1);
        assert_eq!(parse_max_connections("16").unwrap(), 16);
        assert_eq!(parse_max_connections("64").unwrap(), 64);
        for invalid in ["", "0", "65", "1000000", "-1", "4.5", "many"] {
            assert!(parse_max_connections(invalid).is_err(), "{invalid}");
        }
        assert_eq!(MAX_CONNECTIONS, 16);
    }
}

#[cfg(test)]
mod get_body_early_rejection_tests {
    use super::*;

    #[test]
    fn rejects_get_body_without_waiting_for_advertised_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client.write_all(
            b"GET /ping HTTP/1.1\r\nHost: localhost\r\nContent-Length: 65536\r\n\r\n",
        ).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        server.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        assert_eq!(
            read_request(&mut server).err().map(|response| response.status),
            Some("400 Bad Request"),
        );
        assert!(parse_request_line("GET /ping HTTP/1.1").is_ok());
    }
}

#[cfg(test)]
mod expect_header_tests {
    use super::*;

    #[test]
    fn unsupported_expect_does_not_wait_for_the_request_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .write_all(
                b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1024\r\nExpect: 100-continue\r\n\r\n",
            )
            .unwrap();
        let (mut server, _) = listener.accept().unwrap();
        server
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        assert_eq!(
            read_request(&mut server).err().map(|response| response.status),
            Some("417 Expectation Failed"),
        );
    }
}

#[cfg(test)]
mod total_request_deadline_tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Instant;

    #[test]
    fn slow_header_bytes_cannot_extend_total_request_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = std::thread::spawn(move || {
            let mut client = TcpStream::connect(address).unwrap();
            for byte in b"GET /ping HTTP/1.1\\r\\n" {
                if client.write_all(&[*byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(35));
            }
        });
        let (mut server, _) = listener.accept().unwrap();
        let started = Instant::now();
        let response =
            read_request_with_deadline(&mut server, Duration::from_millis(120)).unwrap_err();
        assert_eq!(response.status, "408 Request Timeout");
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(server);
        writer.join().unwrap();
    }

    #[test]
    fn slow_body_bytes_cannot_extend_total_request_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = std::thread::spawn(move || {
            let mut client = TcpStream::connect(address).unwrap();
            client
                .write_all(b"POST / HTTP/1.1\\r\\nContent-Length: 32\\r\\n\\r\\n")
                .unwrap();
            for _ in 0..32 {
                if client.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(35));
            }
        });
        let (mut server, _) = listener.accept().unwrap();
        let response =
            read_request_with_deadline(&mut server, Duration::from_millis(120)).unwrap_err();
        assert_eq!(response.status, "408 Request Timeout");
        drop(server);
        writer.join().unwrap();
    }
}

#[cfg(test)]
mod client_socket_timeout_tests {
    use super::*;

    #[test]
    fn client_connections_receive_read_and_write_timeouts() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        configure_client_timeouts(&server).unwrap();
        assert_eq!(server.read_timeout().unwrap(), Some(Duration::from_secs(5)));
        assert_eq!(server.write_timeout().unwrap(), Some(Duration::from_secs(5)));
        drop(client);
    }
}

#[cfg(test)]
mod response_write_deadline_tests {
    use super::*;

    #[test]
    fn deadline_failure_happens_before_any_network_write() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        let error = write_with_deadline(
            &mut server, b"not sent", Instant::now() - Duration::from_millis(1)
        ).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(client);
    }
}

#[cfg(test)]
mod active_worker_permit_tests {
    use super::*;

    #[test]
    fn dropping_worker_slot_releases_capacity() {
        let active = Arc::new(AtomicUsize::new(1));
        {
            let _permit = ActiveRequest(Arc::clone(&active));
            assert_eq!(active.load(Ordering::Acquire), 1);
        }
        assert_eq!(active.load(Ordering::Acquire), 0);
    }
}

#[cfg(test)]
mod http_metrics_tests {
    use super::*;

    #[test]
    fn metrics_use_stable_prometheus_counter_format() {
        let response = metrics_response(125, 7);
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/plain; version=0.0.4; charset=utf-8");
        let text = std::str::from_utf8(&response.body).unwrap();
        assert!(text.contains("# TYPE rusthouse_http_requests_total counter\n"));
        assert!(text.contains("rusthouse_http_requests_total 125\n"));
        assert!(text.contains("# TYPE rusthouse_http_errors_total counter\n"));
        assert!(text.contains("rusthouse_http_errors_total 7\n"));
    }
}

#[cfg(test)]
mod host_validation_tests {
    use super::*;

    #[test]
    fn accepts_only_bound_address_or_loopback_alias() {
        let loopback: SocketAddr = "127.0.0.1:8123".parse().unwrap();
        for valid in ["127.0.0.1", "127.0.0.1:8123", "localhost", "LOCALHOST:8123"] {
            assert!(host_matches_listener(valid, loopback), "{valid}");
        }
        for bad in [
            "attacker.example", "attacker.example:8123", "127.0.0.1:8124",
            "localhost:8124", "127.0.0.1:8123@evil.example", "",
        ] {
            assert!(!host_matches_listener(bad, loopback), "{bad}");
        }
        let private: SocketAddr = "10.1.2.3:8080".parse().unwrap();
        assert!(host_matches_listener("10.1.2.3:8080", private));
        assert!(!host_matches_listener("localhost:8080", private));
        assert!(!host_matches_listener("10.1.2.4:8080", private));
    }

    #[test]
    fn accepts_bracketed_ipv6_without_disabling_host_validation() {
        let listener: SocketAddr = "[::1]:8123".parse().unwrap();
        for host in ["[::1]", "[::1]:8123", "[0:0:0:0:0:0:0:1]:8123"] {
            assert!(host_matches_listener(host, listener), "{host}");
        }
        for host in [
            "::1", "::1:8123", "[::1]:8124", "[::2]:8123",
            "[::1]garbage", "[::1]:", "[::1]:1234:8123",
            "[localhost]:8123", "[::1", "[::1]]:8123",
        ] {
            assert!(!host_matches_listener(host, listener), "{host}");
        }
        let v4: SocketAddr = "127.0.0.1:8123".parse().unwrap();
        assert!(!host_matches_listener("[::1]:8123", v4));
        assert!(host_matches_listener("localhost:8123", listener));
    }
}

#[cfg(test)]
mod interrupted_request_body_tests {
    use super::*;

    struct InterruptedOnce(bool);

    impl Read for InterruptedOnce {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if !self.0 {
                self.0 = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            buffer[..2].copy_from_slice(b"OK");
            Ok(2)
        }
    }

    #[test]
    fn interrupted_body_read_retries_without_losing_bytes() {
        let mut reader = InterruptedOnce(false);
        let mut result = [0u8; 2];
        assert_eq!(
            read_body_chunk_with_deadline(
                &mut reader, &mut result, Instant::now(), Duration::from_secs(5),
            ).ok().expect("interrupted body read should succeed"),
            2,
        );
        assert_eq!(&result, b"OK");
    }

    #[test]
    fn expired_body_deadline_fails_before_reading() {
        let mut reader = InterruptedOnce(false);
        let mut result = [0u8; 2];
        assert!(read_body_chunk_with_deadline(
            &mut reader, &mut result, Instant::now(), Duration::ZERO,
        ).is_err());
        assert!(!reader.0);
    }
}
