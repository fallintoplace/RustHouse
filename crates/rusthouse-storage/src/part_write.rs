//! Small immutable wide-part writer for local SQL inserts.
//! The caller publishes the completed directory with an atomic rename.

mod encoding;
mod schema;
use self::schema::*;

use self::encoding::*;

use crate::{Column, FileChecksum, PartError, UncompressedChecksum};
use ch_cityhash102::cityhash128;
use naive_cityhash::{U128, cityhash128_with_seed};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

// Larger parts reduce open-file and metadata overhead for bulk imports and OPTIMIZE.
const MAX_PART_ROWS: usize = 262_144;
const GRANULE_ROWS: usize = 1024;
const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;

/// Compression for newly written wide parts. Tiny or incompressible blocks
/// are still stored uncompressed to avoid making them larger.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PartCompression {
    #[default]
    Lz4,
    Zstd,
}

fn invalid(message: impl Into<String>) -> PartError {
    PartError::InvalidScan(message.into())
}

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_owned(),
        source,
    }
}

// Part files and the final checksums manifest are each synced before
// publication. Windows standard file handles cannot flush a directory, so
// require a real directory without following reparse points. The caller's
// atomic rename still publishes the complete part, but power-loss directory
// durability on Windows is weaker than POSIX.
fn checked_part_directory(path: &Path) -> Result<(), PartError> {
    let meta = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if !meta.file_type().is_dir() {
        return Err(invalid("wide-part output is not a real directory"));
    }
    Ok(())
}

#[cfg(windows)]
fn sync_part_directory(path: &Path) -> Result<(), PartError> {
    checked_part_directory(path)
}

#[cfg(not(windows))]
fn sync_part_directory(path: &Path) -> Result<(), PartError> {
    checked_part_directory(path)?;
    fs::File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| io_error(path, error))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), PartError> {
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
fn write_tracked_file(
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

fn append_block(
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
fn block(data: &[u8], compression: PartCompression) -> Result<Vec<u8>, PartError> {
    let mut result = Vec::new();
    append_block(&mut result, data, compression)?;
    Ok(result)
}

// Keep every column stream within its 8 MiB limit while collecting
// granules. Waiting until all granules are materialized can retain hundreds
// of oversized batches before the final stream budget check.
fn retain_bounded_granule(
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

fn validate_part_columns(columns: &[Column]) -> Result<(), PartError> {
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

/// Write a new immutable wide part. This function never replaces an existing path.
/// Each row contains one value per column; None denotes SQL NULL.
/// Empty tables have no data streams but still include a checksums manifest.
pub fn write_wide_part(
    part: &Path,
    columns: &[Column],
    rows: &[Vec<Option<String>>],
) -> Result<(), PartError> {
    write_wide_part_with_compression(part, columns, rows, PartCompression::Lz4)
}

/// Write a wide part with an explicit compression codec. Existing callers
/// continue to use LZ4 through `write_wide_part`.
pub fn write_wide_part_with_compression(
    part: &Path,
    columns: &[Column],
    rows: &[Vec<Option<String>>],
    compression: PartCompression,
) -> Result<(), PartError> {
    if columns.is_empty() || columns.len() > 128 {
        return Err(invalid("part requires 1 to 128 columns"));
    }
    if rows.len() > MAX_PART_ROWS {
        return Err(invalid("a wide part supports at most 262144 rows"));
    }
    validate_part_columns(columns)?;
    if rows.iter().any(|row| row.len() != columns.len()) {
        return Err(invalid("INSERT row width does not match table schema"));
    }
    let granule_count = rows.len().div_ceil(GRANULE_ROWS);
    fs::create_dir(part).map_err(|error| io_error(part, error))?;
    let write_result = (|| -> Result<(), PartError> {
        let mut files = Vec::new();
        let mut definitions = format!("columns format version: 1\n{} columns:\n", columns.len());
        for col in columns {
            definitions.push('`');
            definitions.push_str(&col.name);
            definitions.push_str("` ");
            definitions.push_str(&col.data_type);
            definitions.push('\n');
        }
        write_tracked_file(
            part,
            "columns.txt",
            definitions.as_bytes(),
            None,
            &mut files,
        )?;
        let count = format!("{}\n", rows.len());
        write_tracked_file(part, "count.txt", count.as_bytes(), None, &mut files)?;

        if !rows.is_empty() {
            // Release each column's granules after encoding its streams.
            // Existing marks, compression, checksum and atomic cleanup paths stay unchanged.
            const COLUMN_BATCH: usize = 1;
            for batch_start in (0..columns.len()).step_by(COLUMN_BATCH) {
                let batch_end = (batch_start + COLUMN_BATCH).min(columns.len());
                let mut streams = Vec::with_capacity(COLUMN_BATCH * 3);
                for index in batch_start..batch_end {
                    let col = &columns[index];
                    let (_, nullable) = base_type(&col.data_type);
                    if let Some(inner) = numeric_array_inner_type(&col.data_type) {
                        let mut data_granules = Vec::with_capacity(granule_count);
                        let mut size_granules = Vec::with_capacity(granule_count);
                        let (mut data_bytes, mut size_bytes) = (0, 0);
                        for granule in rows.chunks(GRANULE_ROWS) {
                            let mut data = Vec::new();
                            let mut sizes = Vec::with_capacity(granule.len() * size_of::<u64>());
                            for row in granule {
                                push_numeric_array(
                                    inner,
                                    row[index].as_deref(),
                                    &mut data,
                                    &mut sizes,
                                )?;
                            }
                            retain_bounded_granule(&mut data_granules, &mut data_bytes, data)?;
                            retain_bounded_granule(&mut size_granules, &mut size_bytes, sizes)?;
                        }
                        streams.push((col.name.clone(), data_granules));
                        streams.push((format!("{}.size0", col.name), size_granules));
                        continue;
                    }
                    if let Some(value_type) = map_string_value_type(&col.data_type) {
                        let mut key_granules = Vec::with_capacity(granule_count);
                        let mut value_granules = Vec::with_capacity(granule_count);
                        let mut size_granules = Vec::with_capacity(granule_count);
                        let (mut key_bytes, mut value_bytes, mut size_bytes) = (0, 0, 0);
                        for granule in rows.chunks(GRANULE_ROWS) {
                            let mut keys = Vec::new();
                            let mut values = Vec::new();
                            let mut sizes = Vec::with_capacity(granule.len() * size_of::<u64>());
                            for row in granule {
                                push_string_key_map(
                                    value_type,
                                    row[index].as_deref(),
                                    &mut keys,
                                    &mut values,
                                    &mut sizes,
                                )?;
                            }
                            retain_bounded_granule(&mut key_granules, &mut key_bytes, keys)?;
                            retain_bounded_granule(&mut value_granules, &mut value_bytes, values)?;
                            retain_bounded_granule(&mut size_granules, &mut size_bytes, sizes)?;
                        }
                        streams.push((format!("{}.keys", col.name), key_granules));
                        streams.push((format!("{}.values", col.name), value_granules));
                        streams.push((format!("{}.size0", col.name), size_granules));
                        continue;
                    }
                    let mut data_granules = Vec::with_capacity(granule_count);
                    let mut null_granules = Vec::with_capacity(granule_count);
                    let (mut data_bytes, mut null_bytes) = (0, 0);
                    for granule in rows.chunks(GRANULE_ROWS) {
                        // A fixed-width column's final granule size is known
                        // before encoding. Reserve it once instead of growing at
                        // every row; nullable maps have exactly one byte per row.
                        let mut data = Vec::with_capacity(
                            granule.len() * fixed_width_bytes_per_row(&col.data_type),
                        );
                        let mut nulls =
                            Vec::with_capacity(if nullable { granule.len() } else { 0 });
                        for row in granule {
                            let null = push_cell(&col.data_type, row[index].as_deref(), &mut data)?;
                            if nullable {
                                nulls.push(u8::from(null));
                            }
                        }
                        retain_bounded_granule(&mut data_granules, &mut data_bytes, data)?;
                        if nullable {
                            retain_bounded_granule(&mut null_granules, &mut null_bytes, nulls)?;
                        }
                    }
                    streams.push((col.name.clone(), data_granules));
                    if nullable {
                        streams.push((format!("{}.null", col.name), null_granules));
                    }
                }
                for (name, granules) in streams {
                    // The encoded granules are already in memory. Count bytes
                    // once so the checksum stream does not repeatedly reallocate.
                    let uncompressed_size = granules
                        .iter()
                        .try_fold(0usize, |used, chunk| used.checked_add(chunk.len().max(1)))
                        .filter(|&size| size <= MAX_STREAM_BYTES)
                        .ok_or_else(|| invalid("part column exceeds 8 MiB"))?;
                    let mut encoded = Vec::new();
                    // Hash the decoded stream across granules without retaining a
                    // second full-width copy of the encoded column.
                    let mut decoded_bytes = 0usize;
                    let mut decoded_checksum = StreamingChecksum::new();
                    let mut marks = Vec::with_capacity(24 * granules.len());
                    // Pack adjacent marks into larger compressed blocks, while each
                    // mark retains its exact intra-block decompressed byte offset.
                    const PACKED_BLOCK_BYTES: usize = 64 * 1024;
                    let mut pending = Vec::with_capacity(uncompressed_size.min(PACKED_BLOCK_BYTES));
                    for (index, data) in granules.iter().enumerate() {
                        // Empty Array and Map streams retain an unexposed pad byte.
                        let data: &[u8] = if data.is_empty() { &[0] } else { data };
                        if data.len() > MAX_STREAM_BYTES - decoded_bytes {
                            return Err(invalid("part column exceeds 8 MiB"));
                        }
                        if !pending.is_empty()
                            && data.len() > PACKED_BLOCK_BYTES.saturating_sub(pending.len())
                        {
                            append_block(&mut encoded, &pending, compression)?;
                            pending.clear();
                        }
                        marks.extend_from_slice(&(encoded.len() as u64).to_le_bytes());
                        marks.extend_from_slice(&(pending.len() as u64).to_le_bytes());
                        let row_count = (rows.len() - index * GRANULE_ROWS).min(GRANULE_ROWS);
                        marks.extend_from_slice(&(row_count as u64).to_le_bytes());
                        pending.extend_from_slice(data);
                        decoded_bytes += data.len();
                        decoded_checksum.update(data);
                    }
                    if !pending.is_empty() {
                        append_block(&mut encoded, &pending, compression)?;
                    }
                    write_tracked_file(
                        part,
                        &format!("{name}.bin"),
                        &encoded,
                        Some(UncompressedChecksum {
                            size: decoded_bytes as u64,
                            hash: decoded_checksum.finish(),
                        }),
                        &mut files,
                    )?;
                    write_tracked_file(part, &format!("{name}.mrk2"), &marks, None, &mut files)?;
                }
            }
        }

        // Write the manifest only after all listed files have been synced.
        write_file(&part.join("checksums.txt"), &encode_checksums(&files))?;
        sync_part_directory(part)
    })();
    if write_result.is_err() {
        // A bad value in a later column must not leave a partial part.
        let _ = fs::remove_dir_all(part);
    }
    write_result
}

#[cfg(test)]
#[path = "part_write_tests/tests.rs"]
mod tests;

#[cfg(test)]
mod map_writer_tests {
    use super::*;
    use crate::{
        ArrayOffsetsEncoding, NumericColumn, read_array_offsets, read_legacy_strings,
        read_numeric_column,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn decoded(path: &Path) -> Vec<u8> {
        crate::CompressedBlockReader::open(path)
            .unwrap()
            .next_decoded()
            .unwrap()
            .unwrap()
            .data
    }

    #[test]
    fn writes_native_unbucketed_map_streams_and_preserves_duplicate_keys() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-map-writer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let part = root.join("all_1_1_0");
        let columns = [Column {
            name: "labels".into(),
            data_type: "Map(String, UInt16)".into(),
        }];
        let rows = [
            vec![Some(r#"{"a":1,"a":2,"b":65535}"#.into())],
            vec![Some("{}".into())],
            vec![Some(r#"{"unicode\u0020key":7}"#.into())],
        ];
        write_wide_part(&part, &columns, &rows).unwrap();
        crate::verify_wide_part(&part).unwrap();
        let sizes = decoded(&part.join("labels.size0.bin"));
        let keys = decoded(&part.join("labels.keys.bin"));
        let values = decoded(&part.join("labels.values.bin"));
        let offsets = read_array_offsets(
            &mut sizes.as_slice(),
            rows.len(),
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();
        assert_eq!(offsets.offsets(), &[0, 3, 3, 4]);
        let keys = read_legacy_strings(&mut keys.as_slice(), offsets.element_count()).unwrap();
        assert_eq!(keys.get(0), Some(&b"a"[..]));
        assert_eq!(keys.get(1), Some(&b"a"[..]));
        assert_eq!(keys.get(3), Some(&b"unicode key"[..]));
        let values =
            read_numeric_column(&mut values.as_slice(), "UInt16", offsets.element_count()).unwrap();
        assert_eq!(values, NumericColumn::UInt16(vec![1, 2, 65535, 7]));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_malformed_maps_without_publishing_parts() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-map-invalid-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let columns = [Column {
            name: "labels".into(),
            data_type: "Map(String, UInt8)".into(),
        }];
        for literal in [
            r#"{"a":256}"#,
            r#"{"a":1,}"#,
            r#"{"a":1"#,
            r#"{"a" 1}"#,
            r#"{"a":"not a number"}"#,
            r#"{"bad\uD800":1}"#,
            r#"{"bad\uDC00":1}"#,
            r#"{"a":1} trailing"#,
        ] {
            let path = root.join("reject");
            assert!(
                write_wide_part(&path, &columns, &[vec![Some(literal.into())]]).is_err(),
                "{literal}"
            );
            assert!(!path.exists(), "{literal}");
        }
        assert!(write_wide_part(&root.join("null"), &columns, &[vec![None]]).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
#[path = "part_write_tests/numeric_array_writer_tests.rs"]
mod numeric_array_writer_tests;

#[cfg(test)]
#[path = "part_write_tests/writer_schema_bounds_tests.rs"]
mod writer_schema_bounds_tests;

#[cfg(test)]
#[path = "part_write_tests/append_compressed_block_tests.rs"]
mod append_compressed_block_tests;

#[cfg(test)]
#[path = "part_write_tests/stack_writable_schema_tests.rs"]
mod stack_writable_schema_tests;

#[cfg(test)]
#[path = "part_write_tests/retained_granule_budget_tests.rs"]
mod retained_granule_budget_tests;
