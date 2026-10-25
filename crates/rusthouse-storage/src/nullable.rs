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

    pub fn is_null(&self, index: usize) -> Option<bool> {
        self.null_map.get(index).map(|value| *value != 0)
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
    TooManyRows(usize),
    NullMap(io::Error),
    Numeric(NumericReadError),
    String(StringReadError),
}

impl fmt::Display for NullableReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRows(rows) => write!(f, "Nullable batch exceeds the row limit: {rows}"),
            Self::NullMap(error) => write!(f, "unable to read Nullable null map: {error}"),
            Self::Numeric(error) => write!(f, "{error}"),
            Self::String(error) => write!(f, "{error}"),
        }
    }
}

impl Error for NullableReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TooManyRows(_) => None,
            Self::NullMap(error) => Some(error),
            Self::Numeric(error) => Some(error),
            Self::String(error) => Some(error),
        }
    }
}

fn read_nullable<N: Read, T>(
    nulls: &mut N,
    rows: usize,
    read_values: impl FnOnce() -> Result<T, NullableReadError>,
) -> Result<NullableColumn<T>, NullableReadError> {
    if rows > MAX_BATCH_ROWS {
        return Err(NullableReadError::TooManyRows(rows));
    }

    let mut null_map = vec![0; rows];
    nulls
        .read_exact(&mut null_map)
        .map_err(NullableReadError::NullMap)?;
    let values = read_values()?;
    Ok(NullableColumn { null_map, values })
}

pub fn read_nullable_numeric_column<N: Read, D: Read>(
    nulls: &mut N,
    data: &mut D,
    data_type: &str,
    rows: usize,
) -> Result<NullableColumn<NumericColumn>, NullableReadError> {
    read_nullable(nulls, rows, || {
        read_numeric_column(data, data_type, rows).map_err(NullableReadError::Numeric)
    })
}

pub fn read_nullable_legacy_strings<N: Read, D: Read>(
    nulls: &mut N,
    data: &mut D,
    rows: usize,
) -> Result<NullableColumn<StringColumn>, NullableReadError> {
    read_nullable(nulls, rows, || {
        read_legacy_strings(data, rows).map_err(NullableReadError::String)
    })
}

pub fn read_nullable_sized_strings<N: Read, S: Read, D: Read>(
    nulls: &mut N,
    sizes: &mut S,
    data: &mut D,
    rows: usize,
) -> Result<NullableColumn<StringColumn>, NullableReadError> {
    read_nullable(nulls, rows, || {
        read_sized_strings(sizes, data, rows).map_err(NullableReadError::String)
    })
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
    fn reads_nullable_numeric_rows_without_skipping_nested_defaults() {
        let mut nulls = Bytewise(Cursor::new([0u8, 1, 0, 55]));
        let data = [5u64, 0, u64::MAX]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let mut values = Bytewise(Cursor::new(data));

        let column = read_nullable_numeric_column(&mut nulls, &mut values, "UInt64", 3).unwrap();
        assert_eq!(column.len(), 3);
        assert!(!column.is_empty());
        assert_eq!(column.null_map(), &[0, 1, 0]);
        assert_eq!(
            column.values(),
            &NumericColumn::UInt64(vec![5, 0, u64::MAX])
        );
        assert_eq!(column.is_null(0), Some(false));
        assert_eq!(column.is_null(1), Some(true));
        assert_eq!(column.is_null(2), Some(false));
        assert_eq!(column.is_null(3), None);
        assert_eq!(column.is_null(usize::MAX), None);
        let mut trailing = [0];
        nulls.read_exact(&mut trailing).unwrap();
        assert_eq!(trailing, [55]);
    }

    #[test]
    fn treats_nonzero_null_map_bytes_as_null_like_clickhouse() {
        let mut nulls = &[2u8, 0][..];
        let mut values = &[0xff, 42][..];
        let column = read_nullable_numeric_column(&mut nulls, &mut values, "Int8", 2).unwrap();
        assert_eq!(column.null_map(), &[2, 0]);
        assert_eq!(column.is_null(0), Some(true));
        assert_eq!(column.values(), &NumericColumn::Int8(vec![-1, 42]));
    }

    #[test]
    fn reads_nullable_legacy_strings_with_binary_values() {
        let mut nulls = &[0u8, 1, 0][..];
        let mut values = &[2u8, b'a', 0xff, 0, 1, b'z', 44][..];
        let column = read_nullable_legacy_strings(&mut nulls, &mut values, 3).unwrap();
        assert_eq!(column.values().len(), 3);
        assert_eq!(column.values().get(0), Some(&[b'a', 0xff][..]));
        assert_eq!(column.values().get(1), Some(&b""[..]));
        assert_eq!(column.values().get(2), Some(&b"z"[..]));
        assert_eq!(column.is_null(1), Some(true));
        assert_eq!(values, &[44]);
    }

    #[test]
    fn reads_nullable_sized_strings_with_separate_streams() {
        let mut nulls = &[1u8, 0, 0][..];
        let mut sizes = Vec::new();
        for size in [0u64, 2, 0] {
            sizes.extend_from_slice(&size.to_le_bytes());
        }
        sizes.push(88);
        let mut size_reader = Bytewise(Cursor::new(sizes));
        let mut data_reader = &[0x80u8, 0x00, 99][..];
        let column =
            read_nullable_sized_strings(&mut nulls, &mut size_reader, &mut data_reader, 3).unwrap();

        assert_eq!(column.null_map(), &[1, 0, 0]);
        assert_eq!(column.values().offsets(), &[0, 0, 2, 2]);
        assert_eq!(column.values().get(0), Some(&b""[..]));
        assert_eq!(column.values().get(1), Some(&[0x80, 0][..]));
        assert_eq!(column.values().get(2), Some(&b""[..]));
        assert_eq!(data_reader, &[99]);
        let mut trailing = [0];
        size_reader.read_exact(&mut trailing).unwrap();
        assert_eq!(trailing, [88]);
    }

    #[test]
    fn short_null_map_does_not_read_nested_values() {
        let mut nulls = Bytewise(Cursor::new([0u8]));
        let mut values = &[0u8, 1][..];
        assert!(matches!(
            read_nullable_numeric_column(&mut nulls, &mut values, "UInt8", 2),
            Err(NullableReadError::NullMap(error))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));
        assert_eq!(values, &[0, 1]);
    }

    #[test]
    fn propagates_truncated_numeric_and_string_streams() {
        let mut nulls = &[0u8][..];
        assert!(matches!(
            read_nullable_numeric_column(&mut nulls, &mut &[1u8][..], "UInt64", 1),
            Err(NullableReadError::Numeric(NumericReadError::Io(error)))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));
        let mut nulls = &[0u8][..];
        assert!(matches!(
            read_nullable_legacy_strings(&mut nulls, &mut &[3u8, b'a'][..], 1),
            Err(NullableReadError::String(StringReadError::Io(error)))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));
        let mut nulls = &[0u8][..];
        assert!(matches!(
            read_nullable_sized_strings(&mut nulls, &mut &[1u8][..], &mut io::empty(), 1),
            Err(NullableReadError::String(StringReadError::Io(error)))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn preserves_nested_decoder_type_errors() {
        let mut nulls = &[0u8][..];
        let error =
            read_nullable_numeric_column(&mut nulls, &mut io::empty(), "DateTime", 1).unwrap_err();
        assert!(matches!(
            error,
            NullableReadError::Numeric(NumericReadError::UnsupportedType(_))
        ));
    }

    #[test]
    fn rejects_oversized_batches_before_reading_any_stream() {
        struct NoRead;

        impl Read for NoRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("must reject before reading")
            }
        }

        assert!(matches!(
            read_nullable_numeric_column(&mut NoRead, &mut NoRead, "UInt8", MAX_BATCH_ROWS + 1),
            Err(NullableReadError::TooManyRows(_))
        ));
        assert!(matches!(
            read_nullable_legacy_strings(&mut NoRead, &mut NoRead, MAX_BATCH_ROWS + 1),
            Err(NullableReadError::TooManyRows(_))
        ));
        assert!(matches!(
            read_nullable_sized_strings(&mut NoRead, &mut NoRead, &mut NoRead, MAX_BATCH_ROWS + 1),
            Err(NullableReadError::TooManyRows(_))
        ));
    }

    #[test]
    fn zero_row_batches_do_not_consume_streams() {
        let mut nulls = &[42u8][..];
        let mut data = &[43u8][..];
        let column = read_nullable_numeric_column(&mut nulls, &mut data, "Int16", 0).unwrap();
        assert!(column.is_empty());
        assert_eq!(column.len(), 0);
        assert_eq!(column.values(), &NumericColumn::Int16(Vec::new()));
        assert_eq!(nulls, &[42]);
        assert_eq!(data, &[43]);

        let strings = read_nullable_legacy_strings(&mut io::empty(), &mut io::empty(), 0).unwrap();
        assert!(strings.is_empty());
        let strings =
            read_nullable_sized_strings(&mut io::empty(), &mut io::empty(), &mut io::empty(), 0)
                .unwrap();
        assert!(strings.is_empty());
    }
}
