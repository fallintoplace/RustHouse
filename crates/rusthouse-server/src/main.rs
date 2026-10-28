use rusthouse_server::cluster::{self, Cluster, Peer};
use rusthouse_server::distributed;
use rusthouse_server::health;
use rusthouse_server::{
    Config, MAX_CONNECTIONS, MAX_HEADER_BYTES, MAX_QUERY_BYTES, Response, handle_request,
};
use std::io::{self, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const MAX_CONFIGURED_CONNECTIONS: usize = 64;

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
    "Usage: rusthouse-server [--port <1-65535>] [--part <frozen-part-or-snapshot> | --database <directory>] [--max-rows <n>] [--max-connections <1-64>] [--peer IPv4:<port>]... [--bind <IPv4>] [--allow-private-network]\n\
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
    let words: Vec<&str> = line.split(' ').collect();
    if words.len() != 3
        || words.iter().any(|word| word.is_empty())
        || !matches!(words[2], "HTTP/1.1" | "HTTP/1.0")
        || !words[1].bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(Response::text(
            "400 Bad Request",
            "invalid HTTP request line\n",
        ));
    }
    let (method, target) = (words[0], words[1]);
    if method != "GET" && method != "POST" {
        return Err(Response::text(
            "405 Method Not Allowed",
            "only GET and POST\n",
        ));
    }
    Ok((method, target))
}

fn read_request(stream: &mut TcpStream) -> Result<(String, String, Vec<u8>), Response> {
    let mut input = BufReader::new(stream);
    let mut head = Vec::new();
    // Read only until the headers end, without consuming the beginning of the body.
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8];
        input
            .read_exact(&mut byte)
            .map_err(|_| Response::text("400 Bad Request", "incomplete HTTP request headers\n"))?;
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
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (key, value) = parse_header(line)?;
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
    if size > MAX_QUERY_BYTES {
        return Err(Response::text(
            "413 Payload Too Large",
            "request body too large\n",
        ));
    }
    let mut body = vec![0u8; size];
    input
        .read_exact(&mut body)
        .map_err(|_| Response::text("400 Bad Request", "incomplete HTTP request body\n"))?;
    Ok((method.to_owned(), target.to_owned(), body))
}

fn respond(stream: &mut TcpStream, response: Response) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\
         Connection: close\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)
}

fn serve(mut stream: TcpStream, config: &Config, cluster: &Cluster) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let response = match read_request(&mut stream) {
        Ok((method, target, body)) if target == "/cluster" => {
            cluster::handle(&method, &body, cluster)
        }
        Ok((method, target, body)) if target == "/cluster/health" => {
            health::handle(&method, &body, cluster)
        }
        Ok((method, target, body)) if target == "/distributed" => {
            distributed::handle(&method, &body, cluster, config.max_rows)
        }
        Ok((method, target, body)) => handle_request(&method, &target, &body, config),
        Err(response) => response,
    };
    let _ = respond(&mut stream, response);
}

fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut port = 8123u16;
    let mut part = None;
    let mut database = None;
    let mut max_rows = 10_000usize;
    let mut max_connections = MAX_CONNECTIONS;
    let mut peers = Vec::new();
    let mut bind = Ipv4Addr::LOCALHOST;
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
                    .ok_or_else(|| io::Error::other("missing bind IPv4 address"))?
                    .parse::<Ipv4Addr>()
                    .map_err(|_| io::Error::other("bind address must be numeric IPv4"))?;
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
    if bind != Ipv4Addr::LOCALHOST && (!allow_private_network || !cluster::is_private_address(bind))
    {
        return Err(io::Error::other(
            "non-loopback bind requires --allow-private-network and a private IPv4 address",
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
        .any(|peer| *peer.address().ip() == bind && peer.port() == port)
    {
        return Err(io::Error::other("server cannot be its own peer"));
    }
    let config = Arc::new(Config {
        part,
        database,
        max_rows,
    });
    let listener = TcpListener::bind((bind, port))?;
    eprintln!("RustHouse read-only HTTP listening on http://{bind}:{port}");
    let active = Arc::new(AtomicUsize::new(0));
    for accepted in listener.incoming() {
        let stream = accepted?;
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
            let _ = respond(
                &mut stream,
                Response::text("503 Service Unavailable", "server busy\n"),
            );
            continue;
        }
        let config = Arc::clone(&config);
        let cluster = Arc::clone(&cluster);
        let active = Arc::clone(&active);
        std::thread::spawn(move || {
            serve(stream, &config, &cluster);
            active.fetch_sub(1, Ordering::AcqRel);
        });
    }
    Ok(())
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
