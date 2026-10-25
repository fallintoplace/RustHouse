use crate::{NumericColumn, NumericReadError, StringColumn, StringReadError};
use crate::{read_legacy_strings, read_numeric_column, read_sized_strings};
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

    pub fn is_null(&self, row: usize) -> Option<bool> {
        self.null_map.get(row).map(|value| *value != 0)
    }

    pub fn null_map(&self) -> &[u8] {
        &self.null_map
    }

    pub fn values(&self) -> &T {
        &self.values
    }
}

#[derive(Debug)]
pub enum NullableReadError {
    Io(io::Error),
    TooManyRows(usize),
    InvalidType(String),
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
            Self::InvalidType(data_type) => {
                write!(f, "expected Nullable numeric type, got: {data_type}")
            }
            Self::Numeric(error) => write!(f, "Nullable numeric values: {error}"),
            Self::String(error) => write!(f, "Nullable String values: {error}"),
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

pub fn read_null_map<R: Read>(reader: &mut R, rows: usize) -> Result<Vec<u8>, NullableReadError> {
    if rows > MAX_BATCH_ROWS {
        return Err(NullableReadError::TooManyRows(rows));
    }
    let mut null_map = vec![0; rows];
    reader
        .read_exact(&mut null_map)
        .map_err(NullableReadError::Io)?;
    Ok(null_map)
}

pub fn read_nullable_numeric_column<N: Read, D: Read>(
    nulls: &mut N,
    data: &mut D,
    data_type: &str,
    rows: usize,
) -> Result<NullableColumn<NumericColumn>, NullableReadError> {
    let nested_type = data_type
        .strip_prefix("Nullable(")
        .and_then(|inner| inner.strip_suffix(')'))
        .filter(|inner| !inner.is_empty())
        .ok_or_else(|| NullableReadError::InvalidType(data_type.to_owned()))?;
    let null_map = read_null_map(nulls, rows)?;
    let values =
        read_numeric_column(data, nested_type, rows).map_err(NullableReadError::Numeric)?;
    Ok(NullableColumn { null_map, values })
}

pub fn read_nullable_legacy_strings<N: Read, D: Read>(
    nulls: &mut N,
    data: &mut D,
    rows: usize,
) -> Result<NullableColumn<StringColumn>, NullableReadError> {
    let null_map = read_null_map(nulls, rows)?;
    let values = read_legacy_strings(data, rows).map_err(NullableReadError::String)?;
    Ok(NullableColumn { null_map, values })
}

pub fn read_nullable_sized_strings<N: Read, S: Read, D: Read>(
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
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let count = bytes.len().min(1);
            self.0.read(&mut bytes[..count])
        }
    }

    #[test]
    fn reads_numeric_values_for_every_row_including_nulls() {
        let mut nulls = Cursor::new([0, 1, 255, 77]);
        let mut bytes = Vec::new();
        for value in [42u16, 0, 73] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.push(88);
        let mut data = Cursor::new(bytes);
        let column =
            read_nullable_numeric_column(&mut nulls, &mut data, "Nullable(UInt16)", 3).unwrap();

        assert_eq!(column.len(), 3);
        assert!(!column.is_empty());
        assert_eq!(column.null_map(), &[0, 1, 255]);
        assert_eq!(column.is_null(0), Some(false));
        assert_eq!(column.is_null(1), Some(true));
        assert_eq!(column.is_null(2), Some(true));
        assert_eq!(column.is_null(3), None);
        assert_eq!(column.is_null(usize::MAX), None);
        assert_eq!(column.values(), &NumericColumn::UInt16(vec![42, 0, 73]));
        assert_eq!(nulls.position(), 3);
        assert_eq!(data.position(), 6);
    }

    #[test]
    fn reads_legacy_nullable_strings_without_skipping_null_placeholders() {
        let mut nulls = &[1, 0, 1][..];
        let mut data = &[1, b'x', 2, b'a', 0xff, 0, 33][..];
        let column = read_nullable_legacy_strings(&mut nulls, &mut data, 3).unwrap();

        assert_eq!(column.null_map(), &[1, 0, 1]);
        assert_eq!(column.values().get(0), Some(&b"x"[..]));
        assert_eq!(column.values().get(1), Some(&[b'a', 0xff][..]));
        assert_eq!(column.values().get(2), Some(&b""[..]));
        assert_eq!(data, &[33]);
    }

    #[test]
    fn reads_split_size_nullable_strings_with_short_reads() {
        let mut sizes = Vec::new();
        for length in [0u64, 2, 1] {
            sizes.extend_from_slice(&length.to_le_bytes());
        }
        sizes.push(20);
        let mut nulls = Bytewise(Cursor::new([1, 0, 1, 90]));
        let mut sizes = Bytewise(Cursor::new(sizes));
        let mut data = Bytewise(Cursor::new([b'a', 0, 0xff, 91]));
        let column = read_nullable_sized_strings(&mut nulls, &mut sizes, &mut data, 3).unwrap();

        assert_eq!(column.null_map(), &[1, 0, 1]);
        assert_eq!(column.values().get(0), Some(&b""[..]));
        assert_eq!(column.values().get(1), Some(&[b'a', 0][..]));
        assert_eq!(column.values().get(2), Some(&[0xff][..]));
        assert_eq!(column.values().len(), 3);
    }

    #[test]
    fn rejects_truncated_null_map_before_reading_values() {
        let mut nulls = &[0u8][..];
        let mut data = &[7u8][..];
        let error =
            read_nullable_numeric_column(&mut nulls, &mut data, "Nullable(UInt8)", 2).unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::Io(source) if source.kind() == io::ErrorKind::UnexpectedEof
        ));
        assert_eq!(data, &[7]);
    }

    #[test]
    fn rejects_truncated_nested_values() {
        let error =
            read_nullable_numeric_column(&mut &[0, 1][..], &mut &[10u8][..], "Nullable(UInt16)", 2)
                .unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::Numeric(NumericReadError::Io(source))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));

        let error =
            read_nullable_legacy_strings(&mut &[1][..], &mut &[2, b'x'][..], 1).unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::String(StringReadError::Io(source))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn refuses_excessive_row_counts_without_consuming_streams() {
        struct NoRead;

        impl Read for NoRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("must reject before reading");
            }
        }

        assert!(matches!(
            read_null_map(&mut NoRead, MAX_BATCH_ROWS + 1),
            Err(NullableReadError::TooManyRows(_))
        ));
        assert!(matches!(
            read_nullable_numeric_column(
                &mut NoRead,
                &mut NoRead,
                "Nullable(UInt8)",
                MAX_BATCH_ROWS + 1,
            ),
            Err(NullableReadError::TooManyRows(_))
        ));
        assert!(matches!(
            read_nullable_sized_strings(&mut NoRead, &mut NoRead, &mut NoRead, MAX_BATCH_ROWS + 1,),
            Err(NullableReadError::TooManyRows(_))
        ));
    }

    #[test]
    fn rejects_invalid_or_unsupported_numeric_types() {
        for data_type in ["UInt8", "Nullable()", "Nullable(UInt8", "Nullable(UInt8) "] {
            let error =
                read_nullable_numeric_column(&mut io::empty(), &mut io::empty(), data_type, 0)
                    .unwrap_err();
            assert!(matches!(error, NullableReadError::InvalidType(_)));
        }
        let error =
            read_nullable_numeric_column(&mut io::empty(), &mut io::empty(), "Nullable(String)", 0)
                .unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::Numeric(NumericReadError::UnsupportedType(_))
        ));
    }

    #[test]
    fn reads_empty_batches_without_consuming_streams() {
        let mut nulls = &[1u8][..];
        let mut data = &[44u8][..];
        let column =
            read_nullable_numeric_column(&mut nulls, &mut data, "Nullable(Int32)", 0).unwrap();
        assert!(column.is_empty());
        assert_eq!(column.null_map(), &[]);
        assert_eq!(column.values(), &NumericColumn::Int32(vec![]));
        assert_eq!(nulls, &[1]);
        assert_eq!(data, &[44]);

        let column = read_nullable_legacy_strings(&mut io::empty(), &mut io::empty(), 0).unwrap();
        assert!(column.is_empty());
    }
}
