//! Small immutable wide-part writer for local SQL inserts.
//! The caller publishes the completed directory with an atomic rename.

mod encoding;

use self::encoding::*;

use crate::{Column, FileChecksum, PartError, UncompressedChecksum};
use ch_cityhash102::cityhash128;
use naive_cityhash::{U128, cityhash128_with_seed};
use std::collections::HashSet;
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

fn base_type(kind: &str) -> (&str, bool) {
    if let Some(base) = kind
        .strip_prefix("Nullable(")
        .and_then(|v| v.strip_suffix(')'))
    {
        (base, true)
    } else {
        (kind, false)
    }
}

fn numeric_array_inner_type(kind: &str) -> Option<&str> {
    let inner = kind.strip_prefix("Array(")?.strip_suffix(')')?;
    matches!(
        inner,
        "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
            | "Float32"
            | "Float64"
    )
    .then_some(inner)
}

fn map_string_value_type(kind: &str) -> Option<&str> {
    let value = kind.strip_prefix("Map(String, ")?.strip_suffix(')')?;
    matches!(
        value,
        "String"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
            | "Float32"
            | "Float64"
    )
    .then_some(value)
}

fn supported(kind: &str) -> bool {
    let (base, nullable) = base_type(kind);
    if numeric_array_inner_type(base).is_some() {
        return !nullable;
    }
    if map_string_value_type(base).is_some() {
        return !nullable;
    }
    if base == "UUID" {
        return !nullable;
    }
    matches!(
        base,
        "String"
            | "Date"
            | "Date32"
            | "DateTime"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
            | "Float32"
            | "Float64"
    )
}

fn fixed_width_bytes_per_row(data_type: &str) -> usize {
    match base_type(data_type).0 {
        "UInt8" | "Int8" => 1,
        "UInt16" | "Int16" | "Date" => 2,
        "UInt32" | "Int32" | "Float32" | "Date32" | "DateTime" => 4,
        "UInt64" | "Int64" | "Float64" => 8,
        "UInt128" | "Int128" | "UUID" => 16,
        _ => 0,
    }
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
    uncompressed: Option<&[u8]>,
    files: &mut Vec<FileChecksum>,
) -> Result<(), PartError> {
    write_file(&part.join(name), bytes)?;
    files.push(FileChecksum {
        name: name.to_owned(),
        size: bytes.len() as u64,
        hash: checksum_hash(bytes),
        uncompressed: uncompressed.map(|data| UncompressedChecksum {
            size: data.len() as u64,
            hash: checksum_hash(data),
        }),
    });
    Ok(())
}

fn block(data: &[u8], compression: PartCompression) -> Result<Vec<u8>, PartError> {
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
    let mut result = Vec::with_capacity(16 + 9 + payload.len());
    result.resize(16, 0);
    result.push(method);
    result.extend_from_slice(&(compressed_size + 9).to_le_bytes());
    result.extend_from_slice(&length.to_le_bytes());
    result.extend_from_slice(payload);
    let checksum = u128::from(cityhash128(&result[16..])).to_le_bytes();
    result[..16].copy_from_slice(&checksum);
    Ok(result)
}

fn validate_part_columns(columns: &[Column]) -> Result<(), PartError> {
    let mut names = HashSet::new();
    for col in columns {
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
            || !names.insert(col.name.as_str())
        {
            return Err(invalid(format!("invalid part column name: {}", col.name)));
        }
        if !supported(&col.data_type) {
            return Err(invalid(format!(
                "unsupported writable column type: {}",
                col.data_type
            )));
        }
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
    let mut streams = Vec::with_capacity(columns.len() * 2);
    if !rows.is_empty() {
        for (index, col) in columns.iter().enumerate() {
            let (_, nullable) = base_type(&col.data_type);
            if let Some(inner) = numeric_array_inner_type(&col.data_type) {
                let mut data_granules = Vec::with_capacity(granule_count);
                let mut size_granules = Vec::with_capacity(granule_count);
                for granule in rows.chunks(GRANULE_ROWS) {
                    let mut data = Vec::new();
                    let mut sizes = Vec::with_capacity(granule.len() * size_of::<u64>());
                    for row in granule {
                        push_numeric_array(inner, row[index].as_deref(), &mut data, &mut sizes)?;
                    }
                    data_granules.push(data);
                    size_granules.push(sizes);
                }
                streams.push((col.name.clone(), data_granules));
                streams.push((format!("{}.size0", col.name), size_granules));
                continue;
            }
            if let Some(value_type) = map_string_value_type(&col.data_type) {
                let mut key_granules = Vec::with_capacity(granule_count);
                let mut value_granules = Vec::with_capacity(granule_count);
                let mut size_granules = Vec::with_capacity(granule_count);
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
                    key_granules.push(keys);
                    value_granules.push(values);
                    size_granules.push(sizes);
                }
                streams.push((format!("{}.keys", col.name), key_granules));
                streams.push((format!("{}.values", col.name), value_granules));
                streams.push((format!("{}.size0", col.name), size_granules));
                continue;
            }
            let mut data_granules = Vec::with_capacity(granule_count);
            let mut null_granules = Vec::with_capacity(granule_count);
            for granule in rows.chunks(GRANULE_ROWS) {
                // A fixed-width column's final granule size is known
                // before encoding. Reserve it once instead of growing at
                // every row; nullable maps have exactly one byte per row.
                let mut data =
                    Vec::with_capacity(granule.len() * fixed_width_bytes_per_row(&col.data_type));
                let mut nulls = Vec::with_capacity(if nullable { granule.len() } else { 0 });
                for row in granule {
                    let null = push_cell(&col.data_type, row[index].as_deref(), &mut data)?;
                    if nullable {
                        nulls.push(u8::from(null));
                    }
                }
                data_granules.push(data);
                if nullable {
                    null_granules.push(nulls);
                }
            }
            streams.push((col.name.clone(), data_granules));
            if nullable {
                streams.push((format!("{}.null", col.name), null_granules));
            }
        }
    }

    fs::create_dir(part).map_err(|error| io_error(part, error))?;
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
        for (name, granules) in streams {
            let mut encoded = Vec::new();
            let mut uncompressed = Vec::new();
            let mut marks = Vec::with_capacity(24 * granules.len());
            // Pack adjacent marks into larger compressed blocks, while each
            // mark retains its exact intra-block decompressed byte offset.
            const PACKED_BLOCK_BYTES: usize = 64 * 1024;
            let mut pending = Vec::new();
            for (index, data) in granules.iter().enumerate() {
                // Empty Array and Map streams retain an unexposed pad byte.
                let data: &[u8] = if data.is_empty() { &[0] } else { data };
                if data.len() > MAX_STREAM_BYTES - uncompressed.len() {
                    return Err(invalid("part column exceeds 8 MiB"));
                }
                if !pending.is_empty()
                    && data.len() > PACKED_BLOCK_BYTES.saturating_sub(pending.len())
                {
                    encoded.extend_from_slice(&block(&pending, compression)?);
                    pending.clear();
                }
                marks.extend_from_slice(&(encoded.len() as u64).to_le_bytes());
                marks.extend_from_slice(&(pending.len() as u64).to_le_bytes());
                let row_count = (rows.len() - index * GRANULE_ROWS).min(GRANULE_ROWS);
                marks.extend_from_slice(&(row_count as u64).to_le_bytes());
                pending.extend_from_slice(data);
                uncompressed.extend_from_slice(data);
            }
            if !pending.is_empty() {
                encoded.extend_from_slice(&block(&pending, compression)?);
            }
            write_tracked_file(
                part,
                &format!("{name}.bin"),
                &encoded,
                Some(&uncompressed),
                &mut files,
            )?;
            write_tracked_file(part, &format!("{name}.mrk2"), &marks, None, &mut files)?;
        }
    }

    // Write the manifest only after all listed files have been synced.
    write_file(&part.join("checksums.txt"), &encode_checksums(&files))?;
    fs::File::open(part)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| io_error(part, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        NumericColumn, TypedColumn, TypedPartReader, inspect_part, read_checksums, read_wide_marks,
        verify_part_file_sizes, verify_wide_part,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn root() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-wide-writer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn reserves_exact_numeric_granule_widths() {
        for (kind, width) in [
            ("UInt8", 1),
            ("Nullable(Int16)", 2),
            ("Date", 2),
            ("DateTime", 4),
            ("Float64", 8),
            ("UInt128", 16),
            ("Nullable(String)", 0),
        ] {
            assert_eq!(fixed_width_bytes_per_row(kind), width, "{kind}");
        }
    }

    #[test]
    fn writes_105_columns_and_rejects_129() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-wide-columns-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cols = (0..105)
            .map(|i| Column {
                name: format!("field{i}"),
                data_type: "UInt64".to_owned(),
            })
            .collect::<Vec<_>>();
        let values = vec![Some("3".to_owned()); 105];
        write_wide_part(&root.join("wide"), &cols, &[values]).unwrap();
        assert_eq!(
            crate::inspect_part(root.join("wide"))
                .unwrap()
                .columns
                .len(),
            105
        );
        let too_wide = (0..129)
            .map(|i| Column {
                name: format!("field{i}"),
                data_type: "UInt64".to_owned(),
            })
            .collect::<Vec<_>>();
        assert!(write_wide_part(&root.join("too-wide"), &too_wide, &[]).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compresses_repeating_streams_but_leaves_tiny_blocks_raw() {
        use crate::CompressedBlockReader;
        use std::io::Cursor;

        for (data, codec) in [(vec![7u8; 4096], 0x82u8), (vec![1, 2, 3], 0x02)] {
            let encoded = block(&data, PartCompression::Lz4).unwrap();
            let mut reader =
                CompressedBlockReader::new(Cursor::new(encoded), "column.bin").unwrap();
            let decoded = reader.next_decoded().unwrap().unwrap();
            assert_eq!(decoded.block.method, codec);
            assert_eq!(decoded.data, data);
            assert!(reader.next_decoded().is_none());
        }
    }

    #[test]
    fn block_checksum_covers_the_exact_encoded_payload() {
        for data in [
            vec![1u8, 2, 3, 4],
            vec![0x55u8; 4096],
            (0..4096).map(|index| (index * 73 % 251) as u8).collect(),
        ] {
            let block = block(&data, PartCompression::Lz4).unwrap();
            let checksum = u128::from(cityhash128(&block[16..])).to_le_bytes();
            assert_eq!(&block[..16], &checksum);
            let stored_size = u32::from_le_bytes(block[17..21].try_into().unwrap());
            assert_eq!(stored_size as usize, block.len() - 16);
            let uncompressed_size = u32::from_le_bytes(block[21..25].try_into().unwrap());
            assert_eq!(uncompressed_size as usize, data.len());
        }
    }

    #[test]
    fn round_trip_numeric_text_and_nullable_values() {
        let root = root();
        let part = root.join("all_1_1_0");
        let columns = vec![
            Column {
                name: "id".into(),
                data_type: "Int64".into(),
            },
            Column {
                name: "message".into(),
                data_type: "String".into(),
            },
            Column {
                name: "score".into(),
                data_type: "Nullable(UInt64)".into(),
            },
        ];
        write_wide_part(
            &part,
            &columns,
            &[
                vec![
                    Some("-3".into()),
                    Some("hello\tworld".into()),
                    Some("12".into()),
                ],
                vec![Some("7".into()), Some("مرحبا".into()), None],
            ],
        )
        .unwrap();
        assert_eq!(inspect_part(&part).unwrap().rows, 2);
        let mut reader =
            TypedPartReader::open(&part, &["id".into(), "message".into(), "score".into()]).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        assert!(
            matches!(&batch[0], TypedColumn::Numeric(NumericColumn::Int64(values)) if values == &vec![-3, 7])
        );
        assert!(
            matches!(&batch[1], TypedColumn::Text(values) if values.get(0) == Some(b"hello\tworld".as_slice()) && values.get(1) == Some("مرحبا".as_bytes()))
        );
        assert!(
            matches!(&batch[2], TypedColumn::NullableNumeric { nulls, .. } if nulls == &vec![0,1])
        );
        assert!(reader.next_batch().unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zstd_blocks_round_trip_and_tiny_blocks_stay_uncompressed() {
        use crate::CompressedBlockReader;
        use std::io::Cursor;

        let data = b"long repeated message for ZSTD part streams ".repeat(128);
        let encoded = block(&data, PartCompression::Zstd).unwrap();
        let mut reader = CompressedBlockReader::new(Cursor::new(encoded), "zstd.bin").unwrap();
        let decoded = reader.next_decoded().unwrap().unwrap();
        assert_eq!(decoded.block.method, 0x90);
        assert_eq!(decoded.data, data);
        assert!(reader.next_decoded().is_none());

        let tiny = block(b"tiny", PartCompression::Zstd).unwrap();
        let mut reader = CompressedBlockReader::new(Cursor::new(tiny), "tiny.bin").unwrap();
        let decoded = reader.next_decoded().unwrap().unwrap();
        assert_eq!(decoded.block.method, 0x02);
        assert_eq!(decoded.data, b"tiny");
    }

    #[test]
    fn writes_zstd_wide_parts_with_readable_marks_and_checksums() {
        use crate::CompressedBlockReader;

        let root = root();
        let part = root.join("all_3_3_0");
        let columns = vec![
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "message".into(),
                data_type: "String".into(),
            },
        ];
        let message = "ClickHouse-compatible compression keeps the same column values. ".repeat(4);
        let rows = (0..2050u64)
            .map(|id| vec![Some(id.to_string()), Some(message.clone())])
            .collect::<Vec<_>>();

        write_wide_part_with_compression(&part, &columns, &rows, PartCompression::Zstd).unwrap();
        let mut blocks = CompressedBlockReader::open(part.join("message.bin")).unwrap();
        for _ in 0..3 {
            let decoded = blocks.next_decoded().unwrap().unwrap();
            assert_eq!(decoded.block.method, 0x90);
        }
        assert!(blocks.next_decoded().is_none());

        let marks = read_wide_marks(part.join("message.mrk2")).unwrap();
        assert_eq!(
            marks.marks.iter().map(|mark| mark.rows).collect::<Vec<_>>(),
            vec![Some(1024), Some(1024), Some(2)]
        );
        assert!(marks.marks[0].compressed_offset < marks.marks[1].compressed_offset);
        verify_wide_part(&part).unwrap();

        let mut scan = TypedPartReader::open(&part, &["id".into(), "message".into()]).unwrap();
        for (first, count) in [(0u64, 1024usize), (1024, 1024), (2048, 2)] {
            let batch = scan.next_batch().unwrap().unwrap();
            assert_eq!(batch[0].len(), count);
            assert_eq!(batch[1].len(), count);
            assert!(
                matches!(&batch[0], TypedColumn::Numeric(NumericColumn::UInt64(values))
                if values.first() == Some(&first))
            );
            assert!(matches!(&batch[1], TypedColumn::Text(values)
                if (0..count).all(|index| values.get(index) == Some(message.as_bytes()))));
        }
        assert!(scan.next_batch().unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn computes_clickhouse_blockwise_seeded_manifest_hashes() {
        assert_eq!(checksum_hash(b""), 0);
        // CityHash v1.0.2 with seed (0, 0), not CityHash128.
        assert_eq!(
            checksum_hash(b"abc"),
            0xa78da83cc4e0210684541c08b5db83dcu128
        );
        assert_ne!(checksum_hash(b"abc"), u128::from(cityhash128(b"abc")));

        let one_block = vec![42u8; 2048];
        assert_eq!(
            checksum_hash(&one_block),
            u128::from(cityhash128_with_seed(&one_block, U128::new(0, 0)))
        );

        let input = vec![42u8; 4097];
        let first = cityhash128_with_seed(&input[..2048], U128::new(0, 0));
        let second = cityhash128_with_seed(&input[2048..4096], first);
        let tail = cityhash128_with_seed(&input[4096..], second);
        assert_eq!(checksum_hash(&input), u128::from(tail));
        assert_ne!(checksum_hash(&input), u128::from(cityhash128(&input)));
    }

    #[test]
    fn writes_valid_checksums_for_metadata_and_compressed_streams() {
        let root = root();
        let part = root.join("all_1_1_0");
        let columns = vec![
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "note".into(),
                data_type: "Nullable(String)".into(),
            },
        ];
        write_wide_part(
            &part,
            &columns,
            &[
                vec![Some("10".into()), Some("hi".into())],
                vec![Some("20".into()), None],
            ],
        )
        .unwrap();

        let manifest = read_checksums(part.join("checksums.txt")).unwrap();
        assert_eq!(manifest.version, 3);
        assert_eq!(manifest.files.len(), 8);
        for entry in &manifest.files {
            let bytes = fs::read(part.join(&entry.name)).unwrap();
            assert_eq!(entry.size, bytes.len() as u64);
            assert_eq!(entry.hash, checksum_hash(&bytes));
            assert_eq!(entry.uncompressed.is_some(), entry.name.ends_with(".bin"));
        }

        let id = manifest
            .files
            .iter()
            .find(|file| file.name == "id.bin")
            .unwrap();
        let raw_ids = [10u64, 20]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            id.uncompressed,
            Some(UncompressedChecksum {
                size: raw_ids.len() as u64,
                hash: checksum_hash(&raw_ids),
            })
        );
        let nulls = manifest
            .files
            .iter()
            .find(|file| file.name == "note.null.bin")
            .unwrap();
        assert_eq!(
            nulls.uncompressed,
            Some(UncompressedChecksum {
                size: 2,
                hash: checksum_hash(&[0, 1]),
            })
        );
        assert_eq!(verify_part_file_sizes(&part).unwrap().files, 8);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writes_multiple_aligned_granules_and_reads_each_batch() {
        let root = root();
        let part = root.join("all_2_2_0");
        let cols = vec![
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "note".into(),
                data_type: "Nullable(String)".into(),
            },
        ];
        let rows: Vec<Vec<Option<String>>> = (0..2100)
            .map(|i| vec![Some(i.to_string()), (i % 3 != 0).then(|| format!("row{i}"))])
            .collect();
        write_wide_part(&part, &cols, &rows).unwrap();
        let marks = read_wide_marks(part.join("id.mrk2")).unwrap();
        assert_eq!(
            marks.marks.iter().map(|mark| mark.rows).collect::<Vec<_>>(),
            vec![Some(1024), Some(1024), Some(52)]
        );
        assert_eq!(marks.marks[0].compressed_offset, marks.marks[1].compressed_offset);
        assert!(marks.marks[1].decompressed_offset > marks.marks[0].decompressed_offset);
        let note_marks = read_wide_marks(part.join("note.null.mrk2")).unwrap();
        assert_eq!(
            note_marks
                .marks
                .iter()
                .map(|mark| mark.rows)
                .collect::<Vec<_>>(),
            vec![Some(1024), Some(1024), Some(52)]
        );
        verify_wide_part(&part).unwrap();
        let mut scan = TypedPartReader::open(&part, &["id".into(), "note".into()]).unwrap();
        for (first, count) in [(0u64, 1024usize), (1024, 1024), (2048, 52)] {
            let batch = scan.next_batch().unwrap().unwrap();
            assert_eq!(batch[0].len(), count);
            assert_eq!(batch[1].len(), count);
            assert!(
                matches!(&batch[0], TypedColumn::Numeric(NumericColumn::UInt64(values))
                    if values.first() == Some(&first)
                        && values.last() == Some(&(first + count as u64 - 1)))
            );
            assert!(matches!(&batch[1], TypedColumn::NullableText { nulls, .. }
                if nulls[0] == u8::from(first % 3 == 0)));
        }
        assert!(scan.next_batch().unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_empty_parts_and_rejects_invalid_rows() {
        let root = root();
        let cols = vec![Column {
            name: "id".into(),
            data_type: "UInt8".into(),
        }];
        let empty = root.join("all_0_0_0");
        write_wide_part(&empty, &cols, &[]).unwrap();
        assert_eq!(inspect_part(&empty).unwrap().rows, 0);
        let manifest = read_checksums(empty.join("checksums.txt")).unwrap();
        assert_eq!(
            manifest
                .files
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            vec!["columns.txt", "count.txt"]
        );
        assert_eq!(verify_part_file_sizes(&empty).unwrap().files, 2);
        assert!(write_wide_part(&root.join("bad"), &cols, &[vec![Some("256".into())]]).is_err());
        assert!(!root.join("bad").exists());
        assert!(write_wide_part(&root.join("null"), &cols, &[vec![None]]).is_err());
        assert!(!root.join("null").exists());
        assert!(write_wide_part(&empty, &cols, &[]).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

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
mod numeric_array_writer_tests {
    use super::*;
    use crate::{ArrayOffsetsEncoding, NumericColumn, read_numeric_array_column};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn read_block(path: &Path) -> Vec<u8> {
        let mut reader = crate::CompressedBlockReader::open(path).unwrap();
        reader.next_decoded().unwrap().unwrap().data
    }

    #[test]
    fn writes_clickhouse_numeric_array_element_and_size_streams() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-native-array-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let part = dir.join("all_1_1_0");
        let columns = [Column {
            name: "items".into(),
            data_type: "Array(UInt16)".into(),
        }];
        let rows = [
            vec![Some("[12, 65535]".into())],
            vec![Some("[]".into())],
            vec![Some("[7]".into())],
            vec![Some("[0, 1, 2]".into())],
        ];
        write_wide_part(&part, &columns, &rows).unwrap();
        crate::verify_wide_part(&part).unwrap();
        assert!(part.join("items.size0.bin").exists());
        assert!(part.join("items.size0.mrk2").exists());
        let sizes = read_block(&part.join("items.size0.bin"));
        let data = read_block(&part.join("items.bin"));
        let arrays = read_numeric_array_column(
            &mut sizes.as_slice(),
            &mut data.as_slice(),
            "Array(UInt16)",
            4,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();
        assert_eq!(arrays.offsets(), &[0, 2, 2, 3, 6]);
        assert_eq!(
            arrays.values(),
            &NumericColumn::UInt16(vec![12, 65535, 7, 0, 1, 2])
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn all_empty_array_and_map_granules_have_valid_stream_blocks() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-empty-collection-part-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let part = dir.join("all_1_1_0");
        let columns = [
            Column {
                name: "items".into(),
                data_type: "Array(Int8)".into(),
            },
            Column {
                name: "attrs".into(),
                data_type: "Map(String, UInt8)".into(),
            },
        ];
        let rows = (0..1025)
            .map(|_| vec![Some("[]".into()), Some("{}".into())])
            .collect::<Vec<_>>();
        write_wide_part(&part, &columns, &rows).unwrap();
        crate::verify_wide_part(&part).unwrap();

        let mut reader =
            crate::TypedPartReader::open(&part, &["items".into(), "attrs".into()]).unwrap();
        for expected in [1024, 1] {
            let batch = reader.next_batch().unwrap().unwrap();
            assert_eq!(batch[0].len(), expected);
            assert_eq!(batch[1].len(), expected);
            assert!(matches!(
                &batch[0],
                crate::TypedColumn::Text(values) if values.get(0) == Some(&b"[]"[..])
            ));
            assert!(matches!(
                &batch[1],
                crate::TypedColumn::Text(values) if values.get(0) == Some(&b"{}"[..])
            ));
        }
        assert!(reader.next_batch().unwrap().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_invalid_array_values_before_creating_part() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-invalid-arrays-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let columns = [Column {
            name: "xs".into(),
            data_type: "Array(UInt8)".into(),
        }];
        for value in ["1,2", "[1,]", "[,1]", "[256]", "[NULL]", "[1, [2]]"] {
            let part = dir.join("reject");
            assert!(
                write_wide_part(&part, &columns, &[vec![Some(value.into())]]).is_err(),
                "{value}"
            );
            assert!(!part.exists(), "{value}");
        }
        assert!(write_wide_part(&dir.join("reject-null"), &columns, &[vec![None]]).is_err());
        assert!(
            write_wide_part(
                &dir.join("reject-nested"),
                &[Column {
                    name: "xs".into(),
                    data_type: "Array(Array(UInt8))".into(),
                }],
                &[],
            )
            .is_err()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
