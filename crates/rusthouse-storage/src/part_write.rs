//! Small immutable wide-part writer for local SQL inserts.
//! The caller publishes the completed directory with an atomic rename.
use crate::{Column, FileChecksum, PartError, UncompressedChecksum};
use ch_cityhash102::cityhash128;
use naive_cityhash::{U128, cityhash128_with_seed};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

const MAX_PART_ROWS: usize = 4096;
const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;

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

fn supported(kind: &str) -> bool {
    let (base, _) = base_type(kind);
    matches!(
        base,
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
}

fn push_varuint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn push_cell(kind: &str, value: Option<&str>, output: &mut Vec<u8>) -> Result<bool, PartError> {
    let (base, nullable) = base_type(kind);
    let is_null = value.is_none();
    if is_null && !nullable {
        return Err(invalid(format!("NULL is not permitted for {kind}")));
    }
    let value = value.unwrap_or(if base == "String" { "" } else { "0" });
    macro_rules! numeric {
        ($ty:ty) => {{
            let number = value
                .parse::<$ty>()
                .map_err(|_| invalid(format!("invalid {kind} value: {value}")))?;
            output.extend_from_slice(&number.to_le_bytes());
        }};
    }
    match base {
        "String" => {
            push_varuint(value.len() as u64, output);
            output.extend_from_slice(value.as_bytes());
        }
        "UInt8" => numeric!(u8),
        "UInt16" => numeric!(u16),
        "UInt32" => numeric!(u32),
        "UInt64" => numeric!(u64),
        "UInt128" => numeric!(u128),
        "Int8" => numeric!(i8),
        "Int16" => numeric!(i16),
        "Int32" => numeric!(i32),
        "Int64" => numeric!(i64),
        "Int128" => numeric!(i128),
        "Float32" => {
            let number = value
                .parse::<f32>()
                .map_err(|_| invalid(format!("invalid {kind} value: {value}")))?;
            if !number.is_finite() {
                return Err(invalid("non-finite float values are unsupported"));
            }
            output.extend_from_slice(&number.to_le_bytes());
        }
        "Float64" => {
            let number = value
                .parse::<f64>()
                .map_err(|_| invalid(format!("invalid {kind} value: {value}")))?;
            if !number.is_finite() {
                return Err(invalid("non-finite float values are unsupported"));
            }
            output.extend_from_slice(&number.to_le_bytes());
        }
        _ => return Err(invalid(format!("unsupported writable column type: {kind}"))),
    }
    if output.len() > MAX_STREAM_BYTES {
        return Err(invalid("part column exceeds 8 MiB"));
    }
    Ok(is_null)
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
fn checksum_hash(data: &[u8]) -> u128 {
    const HASH_BLOCK_BYTES: usize = 2048;
    let mut seed = U128::new(0, 0);
    let mut chunks = data.chunks_exact(HASH_BLOCK_BYTES);
    for chunk in chunks.by_ref() {
        seed = cityhash128_with_seed(chunk, seed);
    }
    if !chunks.remainder().is_empty() {
        seed = cityhash128_with_seed(chunks.remainder(), seed);
    }
    seed.into()
}

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

fn encode_checksums(files: &[FileChecksum]) -> Vec<u8> {
    let mut bytes = b"checksums format version: 3\n".to_vec();
    push_varuint(files.len() as u64, &mut bytes);
    for file in files {
        push_varuint(file.name.len() as u64, &mut bytes);
        bytes.extend_from_slice(file.name.as_bytes());
        push_varuint(file.size, &mut bytes);
        bytes.extend_from_slice(&file.hash.to_le_bytes());
        match &file.uncompressed {
            Some(original) => {
                bytes.push(1);
                push_varuint(original.size, &mut bytes);
                bytes.extend_from_slice(&original.hash.to_le_bytes());
            }
            None => bytes.push(0),
        }
    }
    bytes
}

fn block(data: &[u8]) -> Result<Vec<u8>, PartError> {
    let length = u32::try_from(data.len()).map_err(|_| invalid("part column is too large"))?;
    let mut encoded = Vec::with_capacity(25 + data.len());
    encoded.push(0x02); // Uncompressed block, supported by the existing reader.
    encoded.extend_from_slice(&(length + 9).to_le_bytes());
    encoded.extend_from_slice(&length.to_le_bytes());
    encoded.extend_from_slice(data);
    let checksum = u128::from(cityhash128(&encoded)).to_le_bytes();
    let mut result = Vec::with_capacity(16 + encoded.len());
    result.extend_from_slice(&checksum);
    result.extend_from_slice(&encoded);
    Ok(result)
}

/// Write a new immutable wide part. This function never replaces an existing path.
/// Each row contains one value per column; None denotes SQL NULL.
/// Empty tables have no data streams but still include a checksums manifest.
pub fn write_wide_part(
    part: &Path,
    columns: &[Column],
    rows: &[Vec<Option<String>>],
) -> Result<(), PartError> {
    if columns.is_empty() || columns.len() > 64 {
        return Err(invalid("part requires 1 to 64 columns"));
    }
    if rows.len() > MAX_PART_ROWS {
        return Err(invalid("an INSERT part supports at most 4096 rows"));
    }
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

    let mut streams = Vec::new();
    if !rows.is_empty() {
        for (index, col) in columns.iter().enumerate() {
            let mut data = Vec::new();
            let mut nulls = Vec::new();
            let (_, nullable) = base_type(&col.data_type);
            for row in rows {
                if row.len() != columns.len() {
                    return Err(invalid("INSERT row width does not match table schema"));
                }
                let null = push_cell(&col.data_type, row[index].as_deref(), &mut data)?;
                if nullable {
                    nulls.push(u8::from(null));
                }
            }
            streams.push((col.name.clone(), data));
            if nullable {
                streams.push((format!("{}.null", col.name), nulls));
            }
        }
    }

    fs::create_dir(part).map_err(|error| io_error(part, error))?;
    let mut files = Vec::new();
    let mut definitions = format!("columns format version: 1\n{} columns:\n", columns.len());
    for col in columns {
        definitions.push_str(&format!("`{}` {}\n", col.name, col.data_type));
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
        let mut marks = Vec::with_capacity(24);
        marks.extend_from_slice(&0u64.to_le_bytes());
        marks.extend_from_slice(&0u64.to_le_bytes());
        marks.extend_from_slice(&(rows.len() as u64).to_le_bytes());
        for (name, data) in streams {
            let compressed = block(&data)?;
            write_tracked_file(
                part,
                &format!("{name}.bin"),
                &compressed,
                Some(&data),
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
        NumericColumn, TypedColumn, TypedPartReader, inspect_part, read_checksums,
        verify_part_file_sizes,
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
