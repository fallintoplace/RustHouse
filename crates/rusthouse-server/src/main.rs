use rusthouse_server::cluster::{self, Cluster, Peer};
use rusthouse_server::distributed;
use rusthouse_server::health;
use rusthouse_server::{
    Config, MAX_CONNECTIONS, MAX_HEADER_BYTES, MAX_QUERY_BYTES, Response, handle_request,
};
use std::io::{self, BufRead, BufReader, Read, Write};
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
    matches!(
        kind,
        io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted
    )
}

/// Bound concurrent read-only analytical requests to avoid unbounded
/// thread, decoded-block and response-buffer amplification.
fn parse_max_connections(value: &str) -> io::Result<usize> {
    let workers = value
        .parse::<usize>()
        .map_err(|_| io::Error::other("invalid max connections"))?;
    if !(1..=MAX_CONFIGURED_CONNECTIONS).contains(&workers) {
        return Err(io::Error::other("max connections must be 1..64"));
    }
    Ok(workers)
}

fn usage() -> &'static str {
    "Usage: rusthouse-server [--port <1-65535>] [--part <frozen-part-or-snapshot> | --database <directory>] [--max-rows <n>] [--max-connections <1-64>] [--peer IPv4:<port>]... [--bind <IPv4-or-IPv6-loopback>] [--allow-private-network]\n\
     Listens on 127.0.0.1 by default. Private networking requires opt-in. No TLS; set RUSTHOUSE_HTTP_TOKEN for optional bearer authentication.\n"
}

/// Opt-in shared-secret authentication for trusted local deployments.
/// The token must travel only over loopback or an encrypted overlay.
fn valid_configured_token(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'+' | b'/' | b'=')
        })
}

fn bearer_token_matches(expected: &str, authorization: Option<&str>) -> bool {
    let Some(provided) = authorization.and_then(|value| value.strip_prefix("Bearer ")) else {
        return false;
    };
    let expected = expected.as_bytes();
    let provided = provided.as_bytes();
    if expected.len() != provided.len() {
        return false;
    }
    let mut mismatch = 0u8;
    for (&left, &right) in expected.iter().zip(provided) {
        mismatch |= left ^ right;
    }
    mismatch == 0
}

#[cfg(test)]
#[path = "main_tests/http_auth_tests.rs"]
mod http_auth_tests;

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

/// HTTP optional whitespace is SP or HTAB, not Unicode whitespace.
fn http_ows(value: &str) -> &str {
    value.trim_matches([' ', '\t'])
}

fn parse_request_line(line: &str) -> Result<(&str, &str), Response> {
    let mut words = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (words.next(), words.next(), words.next(), words.next())
    else {
        return Err(Response::text(
            "400 Bad Request",
            "invalid HTTP request line\n",
        ));
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

/// Check an HTTP quality value without floating-point parsing. HTTP/1.1
/// quality values have at most three fractional digits and range from 0 to 1.
fn positive_http_quality(value: &str) -> bool {
    let raw = value.as_bytes();
    match raw {
        [b'1'] => true,
        [b'1', b'.', digits @ ..]
            if digits.len() <= 3 && digits.iter().all(|&digit| digit == b'0') =>
        {
            true
        }
        [b'0'] => false,
        [b'0', b'.', digits @ ..] if digits.len() <= 3 && digits.iter().all(u8::is_ascii_digit) => {
            digits.iter().any(|&digit| digit != b'0')
        }
        _ => false,
    }
}

/// Only explicit positive acceptance enables the private peer encoding.
/// Wildcards alone do not negotiate our unregistered, internal LZ4 format.
fn accepts_rusthouse_lz4(header: &str) -> bool {
    let mut accepted = false;
    for entry in header.split(',') {
        let mut parts = entry.split(';');
        let name = parts.next().unwrap_or("").trim_matches([' ', '\t']);
        if !name.eq_ignore_ascii_case("x-rusthouse-lz4") {
            continue;
        }
        let positive = match parts.next() {
            None => true,
            Some(parameter) => {
                let parameter = parameter.trim_matches([' ', '\t']);
                match parameter.split_once('=') {
                    Some((name, value)) if name.eq_ignore_ascii_case("q") => {
                        positive_http_quality(value)
                    }
                    _ => false,
                }
            }
        };
        // Duplicate or malformed preferences must not silently override an
        // explicit rejection. Do not accept extension parameters either.
        if !positive || parts.next().is_some() {
            return false;
        }
        accepted = true;
    }
    accepted
}

/// Advance only through the header terminator. A buffered read may already
/// contain request-body bytes, which must remain available to the body reader.
fn append_header_chunk(head: &mut Vec<u8>, bytes: &[u8]) -> Result<(usize, bool), Response> {
    for (index, &byte) in bytes.iter().enumerate() {
        head.push(byte);
        if head.len() > MAX_HEADER_BYTES {
            return Err(Response::text(
                "413 Payload Too Large",
                "HTTP headers too large\n",
            ));
        }
        if head.ends_with(b"\r\n\r\n") {
            return Ok((index + 1, true));
        }
    }
    Ok((bytes.len(), false))
}

fn read_request_with_deadline(
    stream: &mut TcpStream,
    deadline: Duration,
) -> Result<(String, String, Vec<u8>, bool), Response> {
    let started = Instant::now();
    let mut input = BufReader::new(stream);
    let mut head = Vec::new();
    // Refresh the socket timeout per buffered network read instead of per
    // header byte. Byte-by-byte read_exact previously called setsockopt once
    // for every byte, even for an already-buffered 8 KiB request header.
    loop {
        let remaining = request_remaining(started, deadline)?;
        input
            .get_mut()
            .set_read_timeout(Some(remaining))
            .map_err(|error| request_read_error(error, "cannot set request read timeout\n"))?;
        let (consumed, complete) = {
            let available = input
                .fill_buf()
                .map_err(|error| request_read_error(error, "incomplete HTTP request headers\n"))?;
            if available.is_empty() {
                return Err(Response::text(
                    "400 Bad Request",
                    "incomplete HTTP request headers\n",
                ));
            }
            append_header_chunk(&mut head, available)?
        };
        input.consume(consumed);
        if complete {
            break;
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
    let mut authorization = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (key, value) = parse_header(line)?;
        if key.eq_ignore_ascii_case("host") && host.replace(http_ows(value)).is_some() {
            return Err(Response::text("400 Bad Request", "duplicate Host header\n"));
        }
        if key.eq_ignore_ascii_case("authorization")
            && authorization.replace(http_ows(value)).is_some()
        {
            return Err(Response::text(
                "400 Bad Request",
                "duplicate Authorization header\n",
            ));
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
            let value = http_ows(value);
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
                return Err(Response::text(
                    "400 Bad Request",
                    "duplicate Accept-Encoding\n",
                ));
            }
            seen_accept = true;
            accept_lz4 = accepts_rusthouse_lz4(value);
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
    if let Some(token) = std::env::var_os("RUSTHOUSE_HTTP_TOKEN") {
        let expected = token
            .to_str()
            .filter(|value| valid_configured_token(value))
            .ok_or_else(|| {
                Response::text(
                    "503 Service Unavailable",
                    "invalid server auth configuration\n",
                )
            })?;
        if !bearer_token_matches(expected, authorization) {
            return Err(Response::text(
                "401 Unauthorized",
                "authentication required\n",
            ));
        }
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
        let count =
            read_body_chunk_with_deadline(&mut input, &mut body[received..], started, deadline)?;
        if count == 0 {
            return Err(Response::text(
                "400 Bad Request",
                "incomplete HTTP request body\n",
            ));
        }
        received += count;
    }
    Ok((
        method.to_owned(),
        target.to_owned(),
        body,
        accept_lz4 && method == "POST" && target == "/",
    ))
}

fn read_request(stream: &mut TcpStream) -> Result<(String, String, Vec<u8>, bool), Response> {
    read_request_with_deadline(stream, REQUEST_READ_DEADLINE)
}

/// An interrupted socket write consumes no bytes. The outer loop must
/// recalculate its absolute deadline before attempting the write again.
fn write_response_chunk(writer: &mut impl Write, bytes: &[u8]) -> io::Result<Option<usize>> {
    match writer.write(bytes) {
        Ok(0) => Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "client closed during response",
        )),
        Ok(count) => Ok(Some(count)),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(None),
        Err(error) => Err(error),
    }
}

/// Enforce one total response-write budget, not a fresh timeout per syscall.
fn write_with_deadline(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "response write deadline exceeded")
            })?;
        stream.set_write_timeout(Some(remaining))?;
        // Limit each syscall to 64 KiB and recalculate the budget after it.
        let Some(written) = write_response_chunk(stream, &bytes[..bytes.len().min(64 * 1024)])?
        else {
            continue;
        };
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

// The accept thread must never perform a blocking write. When an overloaded
// client's receive window is full, best-effort delivery is safer than
// preventing other clients from connecting.
const BUSY_RESPONSE: &[u8] = b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 12\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\nserver busy\n";

fn reject_busy_connection(mut stream: TcpStream) {
    if stream.set_nonblocking(true).is_ok() {
        let _ = stream.write_all(BUSY_RESPONSE);
    }
}

#[cfg(test)]
#[path = "main_tests/nonblocking_overload_tests.rs"]
mod nonblocking_overload_tests;

/// Allow IPv6 only on ::1 without extending the experimental
/// unauthenticated peer transport beyond the existing private IPv4 policy.
fn permitted_listener_bind(bind: IpAddr, allow_private_network: bool) -> bool {
    match bind {
        IpAddr::V4(address) => {
            address == Ipv4Addr::LOCALHOST
                || (allow_private_network && cluster::is_private_address(address))
        }
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
                let value = args
                    .next()
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
    eprintln!(
        "RustHouse read-only HTTP listening on http://{}",
        listener.local_addr()?
    );
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
            // A full socket send buffer must never block the accept thread.
            // The ordinary response writer resets the socket timeout to five
            // seconds, which would defeat a short overload deadline.
            reject_busy_connection(stream);
            HTTP_REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);
            HTTP_ERRORS_TOTAL.fetch_add(1, Ordering::Relaxed);
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
#[path = "main_tests/ipv6_bind_policy_tests.rs"]
mod ipv6_bind_policy_tests;

#[cfg(test)]
#[path = "main_tests/buffered_header_boundary_tests.rs"]
mod buffered_header_boundary_tests;

#[cfg(test)]
#[path = "main_tests/http_ows_smuggling_tests.rs"]
mod http_ows_smuggling_tests;

#[cfg(test)]
#[path = "main_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "main_tests/request_line_spacing_regressions.rs"]
mod request_line_spacing_regressions;

#[cfg(test)]
#[path = "main_tests/accept_error_policy_tests.rs"]
mod accept_error_policy_tests;

#[cfg(test)]
#[path = "main_tests/accept_encoding_preferences_tests.rs"]
mod accept_encoding_preferences_tests;

#[cfg(test)]
#[path = "main_tests/compressed_peer_response_tests.rs"]
mod compressed_peer_response_tests;

#[cfg(test)]
#[path = "main_tests/concurrency_capacity_tests.rs"]
mod concurrency_capacity_tests;

#[cfg(test)]
#[path = "main_tests/get_body_early_rejection_tests.rs"]
mod get_body_early_rejection_tests;

#[cfg(test)]
#[path = "main_tests/expect_header_tests.rs"]
mod expect_header_tests;

#[cfg(test)]
#[path = "main_tests/total_request_deadline_tests.rs"]
mod total_request_deadline_tests;

#[cfg(test)]
#[path = "main_tests/client_socket_timeout_tests.rs"]
mod client_socket_timeout_tests;

#[cfg(test)]
#[path = "main_tests/response_write_deadline_tests.rs"]
mod response_write_deadline_tests;

#[cfg(test)]
#[path = "main_tests/active_worker_permit_tests.rs"]
mod active_worker_permit_tests;

#[cfg(test)]
#[path = "main_tests/http_metrics_tests.rs"]
mod http_metrics_tests;

#[cfg(test)]
#[path = "main_tests/host_validation_tests.rs"]
mod host_validation_tests;

#[cfg(test)]
#[path = "main_tests/interrupted_request_body_tests.rs"]
mod interrupted_request_body_tests;

#[cfg(test)]
#[path = "main_tests/interrupted_response_write_tests.rs"]
mod interrupted_response_write_tests;
