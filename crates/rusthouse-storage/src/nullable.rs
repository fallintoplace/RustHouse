use crate::{
    NumericColumn, NumericReadError, StringColumn, StringReadError, read_legacy_strings,
    read_numeric_column, read_sized_strings,
};
use std::error::Error;
use std::fmt;
use std::io::{self, Read};

const MAX_BATCH_ROWS: usize = 1_000_000;

#[derive(Debug, PartialEq)]
pub struct NullableColumn<T> {
    null_map: Vec<u8>,
    values: T,
}

impl<T> NullableColumn<T> {
    pub fn len(&self) -> usize {
        self.null_map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.null_map.is_empty()
    }

    pub fn null_map(&self) -> &[u8] {
        &self.null_map
    }

    pub fn values(&self) -> &T {
        &self.values
    }

    pub fn is_null(&self, row: usize) -> Option<bool> {
        self.null_map.get(row).map(|value| *value != 0)
    }
}

#[derive(Debug)]
pub enum NullableReadError {
    Io(io::Error),
    TooManyRows(usize),
    InvalidNullMap { row: usize, value: u8 },
    Numeric(NumericReadError),
    String(StringReadError),
}

impl fmt::Display for NullableReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "unable to read Nullable null map: {error}"),
            Self::TooManyRows(rows) => {
                write!(f, "Nullable batch exceeds the row limit: {rows}")
            }
            Self::InvalidNullMap { row, value } => {
                write!(f, "invalid Nullable null map value {value} at row {row}")
            }
            Self::Numeric(error) => write!(f, "{error}"),
            Self::String(error) => write!(f, "{error}"),
        }
    }
}

impl Error for NullableReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Numeric(error) => Some(error),
            Self::String(error) => Some(error),
            _ => None,
        }
    }
}

fn read_null_map<R: Read>(reader: &mut R, rows: usize) -> Result<Vec<u8>, NullableReadError> {
    if rows > MAX_BATCH_ROWS {
        return Err(NullableReadError::TooManyRows(rows));
    }

    let mut null_map = vec![0u8; rows];
    reader.read_exact(&mut null_map).map_err(NullableReadError::Io)?;
    for (row, value) in null_map.iter().copied().enumerate() {
        if value > 1 {
            return Err(NullableReadError::InvalidNullMap { row, value });
        }
    }
    Ok(null_map)
}

pub fn read_basic_nullable_numeric<N: Read, D: Read>(
    nulls: &mut N,
    data: &mut D,
    nested_type: &str,
    rows: usize,
) -> Result<NullableColumn<NumericColumn>, NullableReadError> {
    let null_map = read_null_map(nulls, rows)?;
    let values = read_numeric_column(data, nested_type, rows).map_err(NullableReadError::Numeric)?;
    Ok(NullableColumn { null_map, values })
}

pub fn read_basic_nullable_legacy_strings<N: Read, D: Read>(
    nulls: &mut N,
    data: &mut D,
    rows: usize,
) -> Result<NullableColumn<StringColumn>, NullableReadError> {
    let null_map = read_null_map(nulls, rows)?;
    let values = read_legacy_strings(data, rows).map_err(NullableReadError::String)?;
    Ok(NullableColumn { null_map, values })
}

pub fn read_basic_nullable_sized_strings<N: Read, S: Read, D: Read>(
    nulls: &mut N,
    sizes: &mut S,
    data: &mut D,
    rows: usize,
) -> Result<NullableColumn<StringColumn>, NullableReadError> {
    let null_map = read_null_map(nulls, rows)?;
    let values = read_sized_strings(sizes, data, rows).map_err(NullableReadError::String)?;
    Ok(NullableColumn { null_map, values })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Bytewise<R>(R);

    impl<R: Read> Read for Bytewise<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(1);
            self.0.read(&mut buffer[..count])
        }
    }

    struct NoRead;

    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("must reject before reading")
        }
    }

    #[test]
    fn keeps_nested_numeric_values_for_null_rows() {
        let mut nulls = &[0, 1, 0, 1][..];
        let mut data = &[10, 99, 20, 88][..];
        let column = read_basic_nullable_numeric(&mut nulls, &mut data, "UInt8", 4).unwrap();

        assert_eq!(column.len(), 4);
        assert!(!column.is_empty());
        assert_eq!(column.null_map(), &[0, 1, 0, 1]);
        assert_eq!(column.is_null(0), Some(false));
        assert_eq!(column.is_null(1), Some(true));
        assert_eq!(column.is_null(2), Some(false));
        assert_eq!(column.is_null(3), Some(true));
        assert_eq!(column.is_null(4), None);
        assert_eq!(column.is_null(usize::MAX), None);
        assert_eq!(column.values(), &NumericColumn::UInt8(vec![10, 99, 20, 88]));
        assert!(nulls.is_empty());
        assert!(data.is_empty());
    }

    #[test]
    fn handles_short_reads_and_consecutive_numeric_batches() {
        let mut values = Vec::new();
        for value in [-10i16, 200, 300, -40] {
            values.extend_from_slice(&value.to_le_bytes());
        }
        let mut nulls = Bytewise(Cursor::new(vec![1, 0, 0, 1, 77]));
        let mut data = Bytewise(Cursor::new(values));

        let first = read_basic_nullable_numeric(&mut nulls, &mut data, "Int16", 2).unwrap();
        assert_eq!(first.null_map(), &[1, 0]);
        assert_eq!(first.values(), &NumericColumn::Int16(vec![-10, 200]));

        let second = read_basic_nullable_numeric(&mut nulls, &mut data, "Int16", 2).unwrap();
        assert_eq!(second.null_map(), &[0, 1]);
        assert_eq!(second.values(), &NumericColumn::Int16(vec![300, -40]));

        let mut trailing = [0];
        nulls.read_exact(&mut trailing).unwrap();
        assert_eq!(trailing, [77]);
    }

    #[test]
    fn reads_binary_legacy_strings_without_skipping_null_rows() {
        let mut data = &[2, b'a', 0xff, 1, b'x', 0][..];
        let mut nulls = &[0, 1, 0][..];
        let column = read_basic_nullable_legacy_strings(&mut nulls, &mut data, 3).unwrap();

        assert_eq!(column.null_map(), &[0, 1, 0]);
        assert_eq!(column.values().get(0), Some(&[b'a', 0xff][..]));
        assert_eq!(column.values().get(1), Some(&b"x"[..]));
        assert_eq!(column.values().get(2), Some(&b""[..]));
        assert_eq!(column.is_null(1), Some(true));
        assert!(data.is_empty());
    }

    #[test]
    fn reads_sized_strings_with_separate_null_and_size_streams() {
        let mut sizes = Vec::new();
        for size in [3u64, 1, 0] {
            sizes.extend_from_slice(&size.to_le_bytes());
        }
        sizes.push(99);
        let mut nulls = &[0, 1, 0, 77][..];
        let mut size_reader = Bytewise(Cursor::new(sizes));
        let mut data = &[b'a', b'b', b'c', b'x', 42][..];
        let column =
            read_basic_nullable_sized_strings(&mut nulls, &mut size_reader, &mut data, 3).unwrap();

        assert_eq!(column.null_map(), &[0, 1, 0]);
        assert_eq!(column.values().get(0), Some(&b"abc"[..]));
        assert_eq!(column.values().get(1), Some(&b"x"[..]));
        assert_eq!(column.values().get(2), Some(&b""[..]));
        assert_eq!(nulls, &[77]);
        assert_eq!(data, &[42]);
        let mut trailing = [0];
        size_reader.read_exact(&mut trailing).unwrap();
        assert_eq!(trailing, [99]);
    }

    #[test]
    fn rejects_invalid_null_map_without_reading_value_stream() {
        let error =
            read_basic_nullable_numeric(&mut &[0, 2][..], &mut NoRead, "UInt8", 2).unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::InvalidNullMap { row: 1, value: 2 }
        ));
    }

    #[test]
    fn reports_truncated_null_maps_and_nested_values() {
        let error =
            read_basic_nullable_numeric(&mut &[0][..], &mut NoRead, "UInt8", 2).unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::Io(source) if source.kind() == io::ErrorKind::UnexpectedEof
        ));

        let error =
            read_basic_nullable_numeric(&mut &[0][..], &mut io::empty(), "UInt32", 1).unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::Numeric(NumericReadError::Io(source))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));

        let error =
            read_basic_nullable_legacy_strings(&mut &[0][..], &mut &[2, b'a'][..], 1).unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::String(StringReadError::Io(source))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn rejects_unsupported_types_and_untrusted_row_counts() {
        assert!(matches!(
            read_basic_nullable_numeric(&mut &[0][..], &mut NoRead, "Array(UInt8)", 1),
            Err(NullableReadError::Numeric(NumericReadError::UnsupportedType(_)))
        ));
        assert!(matches!(
            read_basic_nullable_numeric(&mut NoRead, &mut NoRead, "UInt8", MAX_BATCH_ROWS + 1),
            Err(NullableReadError::TooManyRows(_))
        ));
        assert!(matches!(
            read_basic_nullable_legacy_strings(&mut NoRead, &mut NoRead, MAX_BATCH_ROWS + 1),
            Err(NullableReadError::TooManyRows(_))
        ));
        assert!(matches!(
            read_basic_nullable_sized_strings(
                &mut NoRead,
                &mut NoRead,
                &mut NoRead,
                MAX_BATCH_ROWS + 1
            ),
            Err(NullableReadError::TooManyRows(_))
        ));
    }

    #[test]
    fn empty_batches_leave_every_stream_untouched() {
        let mut nulls = &[42][..];
        let mut data = &[43][..];
        let column = read_basic_nullable_numeric(&mut nulls, &mut data, "UInt8", 0).unwrap();
        assert!(column.is_empty());
        assert_eq!(column.null_map(), &[]);
        assert_eq!(column.values(), &NumericColumn::UInt8(vec![]));
        assert_eq!(nulls, &[42]);
        assert_eq!(data, &[43]);

        let legacy = read_basic_nullable_legacy_strings(&mut nulls, &mut data, 0).unwrap();
        assert!(legacy.is_empty());
        let mut sizes = &[44][..];
        let sized = read_basic_nullable_sized_strings(&mut nulls, &mut sizes, &mut data, 0).unwrap();
        assert!(sized.is_empty());
        assert_eq!(nulls, &[42]);
        assert_eq!(sizes, &[44]);
        assert_eq!(data, &[43]);
    }
}
