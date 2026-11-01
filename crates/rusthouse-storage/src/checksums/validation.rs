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
    let mut scratch = Vec::new();
    loop {
        let Some(block) = blocks.next_decoded_reusing(scratch) else {
            break;
        };
        let data = block?.data;
        if data.len() > MAX_CHECKSUMS_BYTES as usize - decoded.len() {
            return Err(invalid(path, "decoded checksums exceed 64 MiB"));
        }
        if decoded.is_empty() {
            // One-block manifests use the original decompressed allocation.
            decoded = data;
            scratch = Vec::new();
        } else {
            decoded.extend_from_slice(&data);
            // Large v4 manifests can span hundreds of compressed blocks.
            // Reuse the previous block's allocation on the next decode.
            scratch = data;
        }
    }
    Ok(decoded)
}

// Windows normalizes trailing dots/spaces and treats device names as files
// regardless of their extension. Keep Unix compatibility with existing
// ClickHouse manifests while refusing ambiguous paths on Windows.
#[cfg(any(test, windows))]
fn windows_ambiguous_segment(segment: &str) -> bool {
    if segment.ends_with('.') || segment.ends_with(' ') {
        return true;
    }
    let stem = segment.split('.').next().unwrap_or(segment);
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .iter()
        .any(|name| stem.eq_ignore_ascii_case(name))
    {
        return true;
    }
    let prefix = stem.get(..3);
    let suffix = stem.get(3..);
    prefix
        .is_some_and(|value| value.eq_ignore_ascii_case("COM") || value.eq_ignore_ascii_case("LPT"))
        && suffix.is_some_and(|digit| {
            matches!(
                digit,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

fn valid_path_segment(segment: &str) -> bool {
    if segment.is_empty() || segment == "." || segment == ".." {
        return false;
    }
    #[cfg(windows)]
    if windows_ambiguous_segment(segment) {
        return false;
    }
    true
}

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && !name.contains(':')
        && !name.bytes().any(|byte| byte.is_ascii_control())
        && name.split('/').all(valid_path_segment)
}

pub(super) fn invalid(path: &Path, reason: &'static str) -> PartError {
    PartError::InvalidChecksums {
        path: path.to_path_buf(),
        reason,
    }
}

#[cfg(test)]
mod portable_manifest_path_tests {
    use super::*;

    #[test]
    fn identifies_windows_device_names_without_unicode_slice_panics() {
        for name in [
            "CON",
            "con.txt",
            "PRN",
            "AUX.bin",
            "NUL",
            "com1.dat",
            "COM9",
            "LPT1",
            "lpt9.log",
            "COM¹",
            "LPT².txt",
            "CONIN$",
            "conout$",
            "file.",
            "file ",
            "folder. ",
        ] {
            assert!(windows_ambiguous_segment(name), "{name}");
        }
        for name in [
            "common",
            "CONTEXT",
            "COM0",
            "COM10",
            "LPT0",
            "a.com1",
            "résumé",
            "💾",
            "file.name",
        ] {
            assert!(!windows_ambiguous_segment(name), "{name}");
        }
    }

    #[test]
    fn rejects_unsafe_names_on_the_platform_that_normalizes_them() {
        assert!(valid_name("subdir/data.bin"));
        assert!(!valid_name("../escape"));
        assert!(!valid_name("subdir\\escape"));
        #[cfg(windows)]
        for name in ["NUL", "part/COM1.bin", "part/file.", "part/LPT².dat"] {
            assert!(!valid_name(name), "{name}");
        }
        #[cfg(not(windows))]
        assert!(valid_name("part/COM1.bin"));
    }
}

#[cfg(test)]
mod multiblock_checksums_reuse_tests {
    use super::*;
    use ch_cityhash102::cityhash128;

    fn checked_uncompressed_block(payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x02];
        bytes.extend_from_slice(&(payload.len() as u32 + 9).to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        let mut wire = u128::from(cityhash128(&bytes)).to_le_bytes().to_vec();
        wire.extend_from_slice(&bytes);
        wire
    }

    #[test]
    fn decodes_many_small_blocks_without_changing_byte_order_or_length() {
        let mut wire = Vec::new();
        let mut expected = Vec::new();
        for index in 0..128usize {
            let contents = vec![index as u8; 4096 + (index % 7)];
            wire.extend_from_slice(&checked_uncompressed_block(&contents));
            expected.extend_from_slice(&contents);
        }
        assert_eq!(
            decode_compressed_checksums(&wire, Path::new("checksums.txt")).unwrap(),
            expected
        );
    }

    #[test]
    fn later_bad_block_does_not_return_partial_decoded_manifest() {
        let mut wire = checked_uncompressed_block(&[7u8; 8192]);
        let mut bad = checked_uncompressed_block(b"truncated");
        bad[0] ^= 1;
        wire.extend_from_slice(&bad);
        assert!(decode_compressed_checksums(&wire, Path::new("checksums.txt")).is_err());
    }
}
