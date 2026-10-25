use crate::numeric::{NumericColumn, NumericReadError, read_numeric_column};
use crate::strings::{StringColumn, StringReadError, read_legacy_strings, read_sized_strings};
use std::error::Error;
use std::fmt;
use std::io::{self, Read};

const MAX_NULL_MAP_ROWS: usize = 64 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 16 * 1024;

#[derive(Debug, PartialEq)]
pub struct NullableColumn<T> {
    values: T,
    null_map: Vec<u8>,
}

impl<T> NullableColumn<T> {
    pub fn values(&self) -> &T {
        &self.values
    }

    pub fn null_map(&self) -> &[u8] {
        &self.null_map
    }

    pub fn is_null(&self, row: usize) -> Option<bool> {
        self.null_map.get(row).map(|value| *value != 0)
    }

    pub fn len(&self) -> usize {
        self.null_map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.null_map.is_empty()
    }
}

#[derive(Debug)]
pub enum NullableReadError {
    UnsupportedType(String),
    TooManyRows(usize),
    InvalidNullFlag { row: usize, value: u8 },
    Io(io::Error),
    Numeric(NumericReadError),
    String(StringReadError),
}

impl fmt::Display for NullableReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedType(data_type) => {
                write!(f, "unsupported nullable numeric type: {data_type}")
            }
            Self::TooManyRows(rows) => {
                write!(f, "null map exceeds the 64 MiB row limit: {rows}")
            }
            Self::InvalidNullFlag { row, value } => {
                write!(f, "invalid null flag {value} at row {row}")
            }
            Self::Io(source) => write!(f, "unable to read null map: {source}"),
            Self::Numeric(source) => write!(f, "unable to read nullable numeric data: {source}"),
            Self::String(source) => write!(f, "unable to read nullable String data: {source}"),
        }
    }
}

impl Error for NullableReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(source) => Some(source),
            Self::Numeric(source) => Some(source),
            Self::String(source) => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for NullableReadError {
    fn from(source: io::Error) -> Self {
        Self::Io(source)
    }
}

impl From<NumericReadError> for NullableReadError {
    fn from(source: NumericReadError) -> Self {
        Self::Numeric(source)
    }
}

impl From<StringReadError> for NullableReadError {
    fn from(source: StringReadError) -> Self {
        Self::String(source)
    }
}

pub fn read_null_map<R: Read>(reader: &mut R, rows: usize) -> Result<Vec<u8>, NullableReadError> {
    if rows > MAX_NULL_MAP_ROWS {
        return Err(NullableReadError::TooManyRows(rows));
    }

    let mut null_map = Vec::new();
    let mut buffer = [0u8; READ_BUFFER_BYTES];

    while null_map.len() < rows {
        let count = (rows - null_map.len()).min(buffer.len());
        reader.read_exact(&mut buffer[..count])?;
        if let Some((index, &value)) = buffer[..count]
            .iter()
            .enumerate()
            .find(|(_, value)| **value > 1)
        {
            return Err(NullableReadError::InvalidNullFlag {
                row: null_map.len() + index,
                value,
            });
        }
        null_map.extend_from_slice(&buffer[..count]);
    }
    Ok(null_map)
}

pub fn read_nullable_numeric_column<M: Read, R: Read>(
    null_map_reader: &mut M,
    values_reader: &mut R,
    data_type: &str,
    rows: usize,
) -> Result<NullableColumn<NumericColumn>, NullableReadError> {
    let nested_type = data_type
        .strip_prefix("Nullable(")
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| NullableReadError::UnsupportedType(data_type.to_owned()))?;
    let values = read_numeric_column(values_reader, nested_type, rows)?;
    let null_map = read_null_map(null_map_reader, rows)?;
    Ok(NullableColumn { values, null_map })
}

pub fn read_nullable_legacy_strings<M: Read, R: Read>(
    null_map_reader: &mut M,
    values_reader: &mut R,
    rows: usize,
) -> Result<NullableColumn<StringColumn>, NullableReadError> {
    let values = read_legacy_strings(values_reader, rows)?;
    let null_map = read_null_map(null_map_reader, rows)?;
    Ok(NullableColumn { values, null_map })
}

pub fn read_nullable_sized_strings<M: Read, S: Read, D: Read>(
    null_map_reader: &mut M,
    sizes_reader: &mut S,
    data_reader: &mut D,
    rows: usize,
) -> Result<NullableColumn<StringColumn>, NullableReadError> {
    let values = read_sized_strings(sizes_reader, data_reader, rows)?;
    let null_map = read_null_map(null_map_reader, rows)?;
    Ok(NullableColumn { values, null_map })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_nullable_numeric_values_including_null_placeholders() {
        let mut bytes = Vec::new();
        for value in [42i32, 777, -2] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let mut nulls = Cursor::new([0, 1, 0, 1]);
        let mut values = Cursor::new(bytes);

        let column =
            read_nullable_numeric_column(&mut nulls, &mut values, "Nullable(Int32)", 3).unwrap();

        assert_eq!(column.values(), &NumericColumn::Int32(vec![42, 777, -2]));
        assert_eq!(column.null_map(), &[0, 1, 0]);
        assert_eq!(column.len(), 3);
        assert!(!column.is_empty());
        assert_eq!(column.is_null(0), Some(false));
        assert_eq!(column.is_null(1), Some(true));
        assert_eq!(column.is_null(2), Some(false));
        assert_eq!(column.is_null(3), None);
        assert_eq!(nulls.position(), 3);
        assert_eq!(values.position(), 12);
    }

    #[test]
    fn reads_legacy_nullable_strings_without_dropping_null_placeholders() {
        let mut nulls = Cursor::new([0, 1, 0]);
        let mut values = Cursor::new(b"\x01a\x04skip\x01z");
        let column = read_nullable_legacy_strings(&mut nulls, &mut values, 3).unwrap();

        assert_eq!(column.values().get(0), Some(&b"a"[..]));
        assert_eq!(column.values().get(1), Some(&b"skip"[..]));
        assert_eq!(column.values().get(2), Some(&b"z"[..]));
        assert_eq!(column.is_null(1), Some(true));
        assert_eq!(column.null_map(), &[0, 1, 0]);
    }

    #[test]
    fn reads_split_size_nullable_strings_and_empty_values() {
        let mut sizes = Vec::new();
        for size in [0u64, 3, 1] {
            sizes.extend_from_slice(&size.to_le_bytes());
        }
        let mut nulls = Cursor::new([1, 0, 0]);
        let mut sizes = Cursor::new(sizes);
        let mut data = Cursor::new(b"abcx");

        let column = read_nullable_sized_strings(&mut nulls, &mut sizes, &mut data, 3).unwrap();

        assert_eq!(column.values().get(0), Some(&b""[..]));
        assert_eq!(column.values().get(1), Some(&b"abc"[..]));
        assert_eq!(column.values().get(2), Some(&b"x"[..]));
        assert_eq!(column.is_null(0), Some(true));
        assert_eq!(column.is_null(1), Some(false));
        assert_eq!(data.position(), 4);
    }

    struct OneByteReader<R>(R);

    impl<R: Read> Read for OneByteReader<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(1);
            self.0.read(&mut buffer[..count])
        }
    }

    #[test]
    fn reads_exact_rows_across_short_reads_and_batch_boundaries() {
        let mut reader = OneByteReader(Cursor::new([0, 1, 0, 1, 1]));
        assert_eq!(read_null_map(&mut reader, 2).unwrap(), vec![0, 1]);
        assert_eq!(read_null_map(&mut reader, 2).unwrap(), vec![0, 1]);
        assert_eq!(read_null_map(&mut reader, 1).unwrap(), vec![1]);
    }

    #[test]
    fn rejects_non_boolean_flags_at_the_correct_row() {
        let mut bytes = vec![0u8; READ_BUFFER_BYTES + 1];
        bytes[READ_BUFFER_BYTES] = 2;
        let error = read_null_map(&mut Cursor::new(bytes), READ_BUFFER_BYTES + 1).unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::InvalidNullFlag { row, value: 2 } if row == READ_BUFFER_BYTES
        ));
    }

    #[test]
    fn rejects_truncated_null_map() {
        let error = read_null_map(&mut Cursor::new([0, 1]), 3).unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::Io(source) if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    struct NoRead;

    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("must reject before reading");
        }
    }

    #[test]
    fn rejects_unsupported_numeric_types_before_reading() {
        for name in ["Int32", "Nullable(String)", "Nullable(Nullable(UInt8))"] {
            let mut nulls = NoRead;
            let mut values = NoRead;
            assert!(
                read_nullable_numeric_column(&mut nulls, &mut values, name, 1).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn bounds_allocation_without_reading() {
        assert!(matches!(
            read_null_map(&mut NoRead, MAX_NULL_MAP_ROWS + 1),
            Err(NullableReadError::TooManyRows(_))
        ));
        assert!(matches!(
            read_nullable_numeric_column(
                &mut NoRead,
                &mut NoRead,
                "Nullable(UInt64)",
                MAX_NULL_MAP_ROWS + 1
            ),
            Err(NullableReadError::Numeric(
                NumericReadError::BatchTooLarge { .. }
            ))
        ));
    }

    #[test]
    fn propagates_value_errors_without_consuming_null_map() {
        let mut nulls = NoRead;
        let mut values = Cursor::new([1, 2]);
        assert!(matches!(
            read_nullable_numeric_column(&mut nulls, &mut values, "Nullable(UInt64)", 1),
            Err(NullableReadError::Numeric(NumericReadError::Io(source)))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn handles_empty_batches_without_reading() {
        let numeric =
            read_nullable_numeric_column(&mut NoRead, &mut NoRead, "Nullable(Float32)", 0).unwrap();
        assert_eq!(numeric.len(), 0);
        assert!(numeric.is_empty());

        let strings = read_nullable_legacy_strings(&mut NoRead, &mut NoRead, 0).unwrap();
        assert!(strings.is_empty());
        assert_eq!(strings.is_null(0), None);
    }
}
