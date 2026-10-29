use super::*;

const INVALID_PEER_HTTP_STATUS: &str = "invalid peer HTTP status";

pub(super) fn parse_http_status(status_line: &str) -> Result<u16, String> {
    let mut words = status_line.splitn(3, ' ');
    let version = words.next().ok_or(INVALID_PEER_HTTP_STATUS)?;
    let code = words.next().ok_or(INVALID_PEER_HTTP_STATUS)?;
    let reason = words.next().ok_or(INVALID_PEER_HTTP_STATUS)?;
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0")
        || code.len() != 3
        || !code.bytes().all(|byte| byte.is_ascii_digit())
        || reason.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(INVALID_PEER_HTTP_STATUS.into());
    }
    let status = code
        .parse::<u16>()
        .map_err(|_| INVALID_PEER_HTTP_STATUS)?;
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
) -> Result<(usize, String, bool), String> {
    let mut length = None;
    let mut content_type = None;
    let mut compressed = None;
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
        } else if name.eq_ignore_ascii_case("content-encoding") {
            if compressed.is_some() {
                return Err("duplicate peer Content-Encoding".into());
            }
            if !value.eq_ignore_ascii_case("x-rusthouse-lz4") {
                return Err("unsupported peer transfer encoding".into());
            }
            compressed = Some(true);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err("unsupported peer transfer encoding".into());
        }
    }
    let length = length.ok_or("missing peer Content-Length")?;
    let content_type = content_type.ok_or("missing peer Content-Type")?;
    Ok((length, content_type, compressed.unwrap_or(false)))
}

struct ResponseHead {
    body_start: usize,
    status: u16,
    content_type: String,
    body_length: usize,
    compressed: bool,
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
    let (length, content_type, compressed) = parse_required_headers(lines)?;
    if length > MAX_RESPONSE_BYTES {
        return Err("peer response size mismatch or exceeds 8 MiB".into());
    }
    Ok(Some(ResponseHead {
        body_start: head_end + 4,
        status,
        content_type,
        body_length: length,
        compressed,
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

#[cfg(test)]
pub(super) fn parse_response(raw: &[u8]) -> Result<PeerResponse, String> {
    let head = response_head(raw)?.ok_or("peer response has no header terminator")?;
    if raw.len() != head.body_start + head.body_length {
        return Err("peer response size mismatch or exceeds 8 MiB".into());
    }
    let body = if head.compressed {
        let compressed = &raw[head.body_start..];
        if compressed.len() < 4 {
            return Err("truncated compressed peer response".into());
        }
        // Bound the uncompressed size before any allocation. The wire length
        // alone cannot protect against a decompression bomb.
        let size = u32::from_le_bytes(compressed[..4].try_into().unwrap()) as usize;
        if size > MAX_RESPONSE_BYTES {
            return Err("decompressed peer response exceeds 16 MiB".into());
        }
        lz4_flex::decompress_size_prepended(compressed)
            .map_err(|_| "invalid compressed peer response".to_owned())?
    } else {
        raw[head.body_start..].to_vec()
    };
    Ok(PeerResponse {
        status: head.status,
        content_type: head.content_type,
        body,
    })
}

/// Decode a response when the caller owns the network buffer. Plain bodies
/// are shifted in place rather than cloning up to 16 MiB of uncompressed TSV.
pub(super) fn parse_response_owned(mut raw: Vec<u8>) -> Result<PeerResponse, String> {
    let head = response_head(&raw)?.ok_or("peer response has no header terminator")?;
    if raw.len() != head.body_start + head.body_length {
        return Err("peer response size mismatch or exceeds 8 MiB".into());
    }
    let body = if head.compressed {
        let compressed = &raw[head.body_start..];
        if compressed.len() < 4 {
            return Err("truncated compressed peer response".into());
        }
        let size = u32::from_le_bytes(compressed[..4].try_into().unwrap()) as usize;
        if size > MAX_RESPONSE_BYTES {
            return Err("decompressed peer response exceeds 16 MiB".into());
        }
        lz4_flex::decompress_size_prepended(compressed)
            .map_err(|_| "invalid compressed peer response".to_owned())?
    } else {
        raw.copy_within(head.body_start.., 0);
        raw.truncate(head.body_length);
        raw
    };
    Ok(PeerResponse {
        status: head.status,
        content_type: head.content_type,
        body,
    })
}

#[cfg(test)]
mod owned_response_parser_tests {
    use super::*;

    #[test]
    fn accepts_uppercase_lz4_content_encoding() {
        let body = b"compressed peer payload";
        let encoded = lz4_flex::compress_prepend_size(body);
        let mut wire = format!(
            "HTTP/1.1 200 OK\\r\\nContent-Length: {}\\r\\nContent-Type: text/plain\\r\\nContent-Encoding: X-RUSTHOUSE-LZ4\\r\\n\\r\\n",
            encoded.len(),
        ).into_bytes();
        wire.extend_from_slice(&encoded);
        assert_eq!(parse_response_owned(wire).unwrap().body, body);
    }

    #[test]
    fn plain_peer_response_reuses_the_original_wire_allocation() {
        let body = vec![b'q'; 1024 * 1024];
        let mut wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/tab-separated-values; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
            body.len(),
        ).into_bytes();
        wire.extend_from_slice(&body);
        let allocation = wire.as_ptr();
        let parsed = parse_response_owned(wire).unwrap();
        assert_eq!(parsed.body.as_ptr(), allocation);
        assert_eq!(parsed.body, body);
    }

    #[test]
    fn owned_and_borrowed_decoding_reject_truncated_bodies_equally() {
        let wire = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nabc";
        assert!(parse_response(wire).is_err());
        assert!(parse_response_owned(wire.to_vec()).is_err());
    }

    #[test]
    fn owned_parser_preserves_lz4_decompression() {
        let body = vec![b'a'; 9000];
        let encoded = lz4_flex::compress_prepend_size(&body);
        let mut wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Encoding: x-rusthouse-lz4\r\nContent-Length: {}\r\n\r\n",
            encoded.len(),
        ).into_bytes();
        wire.extend_from_slice(&encoded);
        assert_eq!(parse_response_owned(wire).unwrap().body, body);
    }
}

#[cfg(test)]
mod compressed_response_tests {
    use super::*;

    #[test]
    fn decodes_negotiated_large_lz4_response_with_exact_content_length() {
        let plain = vec![b'e'; 512 * 1024];
        let compressed = lz4_flex::compress_prepend_size(&plain);
        let mut wire = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/tab-separated-values; charset=utf-8\r\nContent-Encoding: x-rusthouse-lz4\r\n\r\n",
            compressed.len(),
        ).into_bytes();
        wire.extend_from_slice(&compressed);
        assert_eq!(expected_wire_length(&wire).unwrap(), Some(wire.len()));
        assert_eq!(parse_response(&wire).unwrap().body, plain);
    }

    #[test]
    fn compressed_transport_rejects_bombs_and_unknown_encodings() {
        let mut bomb = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Type: text/plain\r\nContent-Encoding: x-rusthouse-lz4\r\n\r\n".to_vec();
        bomb.extend_from_slice(&((MAX_RESPONSE_BYTES + 1) as u32).to_le_bytes());
        assert!(parse_response(&bomb).is_err());
        let unknown = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: text/plain\r\nContent-Encoding: gzip\r\n\r\nOK";
        assert!(parse_response(unknown).is_err());
    }
}
