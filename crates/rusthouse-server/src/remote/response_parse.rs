use super::*;

pub(super) fn parse_http_status(status_line: &str) -> Result<u16, String> {
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
    Ok(status)
}

pub(super) fn parse_peer_header(line: &str) -> Result<(&str, &str), String> {
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
    Ok((name, value.trim()))
}

pub(super) fn parse_content_length(value: &str) -> Result<usize, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalid peer Content-Length".into());
    }
    value
        .parse::<usize>()
        .map_err(|_| "invalid peer Content-Length".into())
}

fn parse_required_headers<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> Result<(usize, String), String> {
    let mut length = None;
    let mut content_type = None;
    for line in lines {
        let (name, value) = parse_peer_header(line)?;
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err("invalid peer Content-Length".into());
            }
            length = Some(parse_content_length(value)?);
        } else if name.eq_ignore_ascii_case("content-type") {
            if content_type.is_some() {
                return Err("duplicate peer Content-Type".into());
            }
            if value.is_empty() {
                return Err("empty peer Content-Type".into());
            }
            content_type = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("content-encoding")
        {
            return Err("unsupported peer transfer encoding".into());
        }
    }
    let length = length.ok_or("missing peer Content-Length")?;
    let content_type = content_type.ok_or("missing peer Content-Type")?;
    Ok((length, content_type))
}

struct ResponseHead {
    body_start: usize,
    status: u16,
    content_type: String,
    body_length: usize,
}

/// Parse a complete HTTP response head without waiting for the peer to close
/// its connection. The advertised body length is untrusted and remains bounded.
fn response_head(raw: &[u8]) -> Result<Option<ResponseHead>, String> {
    let Some(head_end) = raw.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
        if raw.len() >= MAX_HEADER_BYTES {
            return Err("peer headers exceed 8 KiB".into());
        }
        return Ok(None);
    };
    if head_end + 4 > MAX_HEADER_BYTES {
        return Err("peer headers exceed 8 KiB".into());
    }
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| "peer headers are not UTF-8")?;
    let mut lines = head.split("\r\n");
    let status = parse_http_status(lines.next().ok_or("missing peer status")?)?;
    let (length, content_type) = parse_required_headers(lines)?;
    if length > MAX_RESPONSE_BYTES {
        return Err("peer response size mismatch or exceeds 8 MiB".into());
    }
    Ok(Some(ResponseHead {
        body_start: head_end + 4,
        status,
        content_type,
        body_length: length,
    }))
}

/// Total valid wire bytes needed for a complete content-length response.
pub(super) fn expected_wire_length(raw: &[u8]) -> Result<Option<usize>, String> {
    response_head(raw)?
        .map(|head| {
            head.body_start
                .checked_add(head.body_length)
                .ok_or("peer response size overflow".into())
        })
        .transpose()
}

pub(super) fn parse_response(raw: &[u8]) -> Result<PeerResponse, String> {
    let head = response_head(raw)?.ok_or("peer response has no header terminator")?;
    if raw.len() != head.body_start + head.body_length {
        return Err("peer response size mismatch or exceeds 8 MiB".into());
    }
    Ok(PeerResponse {
        status: head.status,
        content_type: head.content_type,
        body: raw[head.body_start..].to_vec(),
    })
}

/// Consume a validated peer response without allocating a second body buffer.
/// On large distributed reads, copying each 16 MiB response doubles peak
/// in-flight receive allocations per peer.
pub(super) fn parse_response_owned(mut raw: Vec<u8>) -> Result<PeerResponse, String> {
    let head = response_head(&raw)?.ok_or("peer response has no header terminator")?;
    if raw.len() != head.body_start + head.body_length {
        return Err("peer response size mismatch or exceeds 8 MiB".into());
    }
    raw.copy_within(head.body_start.., 0);
    raw.truncate(head.body_length);
    Ok(PeerResponse {
        status: head.status,
        content_type: head.content_type,
        body: raw,
    })
}

#[cfg(test)]
mod owned_response_tests {
    use super::*;

    #[test]
    fn moves_large_body_into_existing_allocation_without_new_buffer() {
        let payload = vec![b'x'; 2 * 1024 * 1024];
        let mut raw = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",
            payload.len()
        ).into_bytes();
        raw.extend_from_slice(&payload);
        let pointer = raw.as_ptr();
        let parsed = parse_response_owned(raw).unwrap();
        assert_eq!(parsed.body.as_ptr(), pointer);
        assert_eq!(parsed.body, payload);
        assert_eq!(parsed.content_type, "text/plain");
    }

    #[test]
    fn owned_and_borrowed_parsing_share_validation() {
        let valid = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: text/plain\r\n\r\nOK";
        assert_eq!(parse_response_owned(valid.to_vec()).unwrap(), parse_response(valid).unwrap());
        for bad in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Type: text/plain\r\n\r\nOK".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\nContent-Type: text/plain\r\n\r\nOK",
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: text/plain\r\n\r\nOKX",
        ] {
            assert!(parse_response_owned(bad.to_vec()).is_err());
        }
    }
}
