use super::*;

// Part files and the final checksums manifest are each synced before
// publication. Windows standard file handles cannot flush a directory, so
// require a real directory without following reparse points. The caller's
// atomic rename still publishes the complete part, but power-loss directory
// durability on Windows is weaker than POSIX.
pub(super) fn checked_part_directory(path: &Path) -> Result<(), PartError> {
    let meta = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if !meta.file_type().is_dir() {
        return Err(invalid("wide-part output is not a real directory"));
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn sync_part_directory(path: &Path) -> Result<(), PartError> {
    checked_part_directory(path)
}

#[cfg(not(windows))]
pub(super) fn sync_part_directory(path: &Path) -> Result<(), PartError> {
    checked_part_directory(path)?;
    fs::File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| io_error(path, error))
}

pub(super) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), PartError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error(path, error))?;
    file.write_all(bytes)
        .map_err(|error| io_error(path, error))?;
    file.sync_all().map_err(|error| io_error(path, error))
}

/// ClickHouse HashingWriteBuffer hashes 2048-byte blocks with CityHash128WithSeed,
/// starting with a zero seed. This differs from compressed block checksums.
pub(super) fn write_tracked_file(
    part: &Path,
    name: &str,
    bytes: &[u8],
    uncompressed: Option<UncompressedChecksum>,
    files: &mut Vec<FileChecksum>,
) -> Result<(), PartError> {
    write_file(&part.join(name), bytes)?;
    files.push(FileChecksum {
        name: name.to_owned(),
        size: bytes.len() as u64,
        hash: checksum_hash(bytes),
        uncompressed,
    });
    Ok(())
}

pub(super) fn append_block(
    result: &mut Vec<u8>,
    data: &[u8],
    compression: PartCompression,
) -> Result<(), PartError> {
    let length = u32::try_from(data.len()).map_err(|_| invalid("part column is too large"))?;
    // Keep tiny and incompressible blocks in the ClickHouse NONE format.
    let compressed = if data.len() >= 128 {
        Some(match compression {
            PartCompression::Lz4 => (0x82, lz4_flex::block::compress(data)),
            PartCompression::Zstd => (
                0x90,
                zstd::bulk::compress(data, 3)
                    .map_err(|error| invalid(format!("ZSTD compression failed: {error}")))?,
            ),
        })
    } else {
        None
    };
    let (method, payload) = match compressed.as_ref() {
        Some((method, bytes)) if bytes.len().saturating_add(16) < data.len() => {
            (*method, bytes.as_slice())
        }
        _ => (0x02, data),
    };
    let compressed_size =
        u32::try_from(payload.len()).map_err(|_| invalid("part column is too large"))?;
    // Reserve the checksum prefix and write the checked header and payload
    // directly into the final buffer, avoiding a second allocation and copy.
    let start = result.len();
    result.reserve(16 + 9 + payload.len());
    result.resize(start + 16, 0);
    result.push(method);
    result.extend_from_slice(&(compressed_size + 9).to_le_bytes());
    result.extend_from_slice(&length.to_le_bytes());
    result.extend_from_slice(payload);
    let checksum = u128::from(cityhash128(&result[start + 16..])).to_le_bytes();
    result[start..start + 16].copy_from_slice(&checksum);
    Ok(())
}

#[cfg(test)]
pub(super) fn block(data: &[u8], compression: PartCompression) -> Result<Vec<u8>, PartError> {
    let mut result = Vec::new();
    append_block(&mut result, data, compression)?;
    Ok(result)
}

// Keep every column stream within its 8 MiB limit while collecting
// granules. Waiting until all granules are materialized can retain hundreds
// of oversized batches before the final stream budget check.
pub(super) fn retain_bounded_granule(
    granules: &mut Vec<Vec<u8>>,
    used: &mut usize,
    data: Vec<u8>,
) -> Result<(), PartError> {
    // Empty variable-length granules contribute a physical pad byte.
    let next = used
        .checked_add(data.len().max(1))
        .filter(|&bytes| bytes <= MAX_STREAM_BYTES)
        .ok_or_else(|| invalid("part column exceeds 8 MiB"))?;
    *used = next;
    granules.push(data);
    Ok(())
}

pub(super) fn validate_part_columns(columns: &[Column]) -> Result<(), PartError> {
    // Match the catalog and scan limits for all public wide-part writer callers.
    if columns.is_empty() || columns.len() > 128 {
        return Err(invalid("wide-part schema must contain 1..128 columns"));
    }
    // The writable schema has at most 128 columns. Keep borrowed names
    // in bounded stack storage instead of allocating a hash table per part.
    let mut names = [""; 128];
    for (index, col) in columns.iter().enumerate() {
        if col.name.is_empty()
            || col.name.len() > 128
            || !matches!(
                col.name.as_bytes().first(),
                Some(b'a'..=b'z' | b'A'..=b'Z' | b'_')
            )
            || !col
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(invalid(format!("invalid part column name: {}", col.name)));
        }
        if !supported(&col.data_type) {
            return Err(invalid(format!(
                "unsupported writable column type: {}",
                col.data_type
            )));
        }
        names[index] = col.name.as_str();
    }
    let names = &mut names[..columns.len()];
    names.sort_unstable();
    if let Some(duplicate) = names.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(invalid(format!(
            "invalid part column name: {}",
            duplicate[0]
        )));
    }
    Ok(())
}
