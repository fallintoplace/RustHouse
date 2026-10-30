use super::*;

pub(super) fn checksums_io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_path_buf(),
        source,
    }
}

pub(super) fn parse_checksum_header<'a>(
    content: &'a [u8],
    path: &Path,
) -> Result<(u8, &'a [u8]), PartError> {
    let header = content
        .strip_prefix(HEADER)
        .ok_or_else(|| invalid(path, "missing format header"))?;
    let end = header
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| invalid(path, "unterminated format header"))?;
    let version = match &header[..end] {
        b"3" => 3,
        b"4" => 4,
        _ => return Err(invalid(path, "unsupported checksums format version")),
    };
    Ok((version, &header[end + 1..]))
}

pub(super) fn decode_compressed_checksums(
    payload: &[u8],
    path: &Path,
) -> Result<Vec<u8>, PartError> {
    let mut decoded = Vec::new();
    let mut blocks = CompressedBlockReader::new(Cursor::new(payload), path)?;
    while let Some(block) = blocks.next_decoded() {
        let block = block?;
        if block.data.len() > MAX_CHECKSUMS_BYTES as usize - decoded.len() {
            return Err(invalid(path, "decoded checksums exceed 64 MiB"));
        }
        if decoded.is_empty() {
            // A single compressed checksums block is common. Take ownership
            // of its decoded allocation instead of copying its entire body.
            decoded = block.data;
        } else {
            decoded.extend_from_slice(&block.data);
        }
    }
    Ok(decoded)
}

fn valid_path_segment(segment: &str) -> bool {
    !segment.is_empty() && segment != "." && segment != ".."
}

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && !name.contains(':')
        && !name.bytes().any(|byte| byte.is_ascii_control())
        && name
            .split('/')
            .all(valid_path_segment)
        && !name.starts_with('/')
}

pub(super) fn invalid(path: &Path, reason: &'static str) -> PartError {
    PartError::InvalidChecksums {
        path: path.to_path_buf(),
        reason,
    }
}
