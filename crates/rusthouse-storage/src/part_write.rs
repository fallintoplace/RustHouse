//! Small immutable wide-part writer for local SQL inserts.
//! The caller publishes the completed directory with an atomic rename.
use crate::{Column, PartError};
use ch_cityhash102::cityhash128;
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
/// For empty tables only count.txt and columns.txt are needed.
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
    let mut definitions = format!("columns format version: 1\n{} columns:\n", columns.len());
    for col in columns {
        definitions.push_str(&format!("`{}` {}\n", col.name, col.data_type));
    }
    write_file(&part.join("columns.txt"), definitions.as_bytes())?;
    write_file(
        &part.join("count.txt"),
        format!("{}\n", rows.len()).as_bytes(),
    )?;

    if !rows.is_empty() {
        let mut marks = Vec::with_capacity(24);
        marks.extend_from_slice(&0u64.to_le_bytes());
        marks.extend_from_slice(&0u64.to_le_bytes());
        marks.extend_from_slice(&(rows.len() as u64).to_le_bytes());
        for (name, data) in streams {
            write_file(&part.join(format!("{name}.bin")), &block(&data)?)?;
            write_file(&part.join(format!("{name}.mrk2")), &marks)?;
        }
    }
    fs::File::open(part)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| io_error(part, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NumericColumn, TypedColumn, TypedPartReader, inspect_part};
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
    fn accepts_empty_parts_and_rejects_invalid_rows() {
        let root = root();
        let cols = vec![Column {
            name: "id".into(),
            data_type: "UInt8".into(),
        }];
        let empty = root.join("all_0_0_0");
        write_wide_part(&empty, &cols, &[]).unwrap();
        assert_eq!(inspect_part(&empty).unwrap().rows, 0);
        assert!(write_wide_part(&root.join("bad"), &cols, &[vec![Some("256".into())]]).is_err());
        assert!(!root.join("bad").exists());
        assert!(write_wide_part(&root.join("null"), &cols, &[vec![None]]).is_err());
        assert!(!root.join("null").exists());
        assert!(write_wide_part(&empty, &cols, &[]).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
