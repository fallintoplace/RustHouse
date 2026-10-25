use crate::{NumericColumn, NumericReadError, read_numeric_column};
use std::error::Error;
use std::fmt;
use std::io::{self, Read};

const MAX_NULL_MAP_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub struct NullableNumericColumn {
    values: NumericColumn,
    null_map: Vec<u8>,
}

impl NullableNumericColumn {
    pub fn len(&self) -> usize {
        self.null_map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.null_map.is_empty()
    }

    pub fn values(&self) -> &NumericColumn {
        &self.values
    }

    pub fn null_map(&self) -> &[u8] {
        &self.null_map
    }

    pub fn is_null(&self, row: usize) -> Option<bool> {
        self.null_map.get(row).map(|value| *value != 0)
    }
}

#[derive(Debug)]
pub enum NullableNumericReadError {
    UnsupportedType(String),
    TooManyRows(usize),
    NullMap(io::Error),
    Nested(NumericReadError),
}

impl fmt::Display for NullableNumericReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedType(data_type) => {
                write!(f, "expected Nullable(numeric type), got {data_type}")
            }
            Self::TooManyRows(rows) => {
                write!(f, "{rows} rows exceed the 64 MiB null-map limit")
            }
            Self::NullMap(source) => write!(f, "unable to read null map: {source}"),
            Self::Nested(source) => write!(f, "unable to read nullable values: {source}"),
        }
    }
}

impl Error for NullableNumericReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NullMap(source) => Some(source),
            Self::Nested(source) => Some(source),
            _ => None,
        }
    }
}

pub fn read_nullable_numeric_column<M: Read, D: Read>(
    null_map: &mut M,
    data: &mut D,
    data_type: &str,
    rows: usize,
) -> Result<NullableNumericColumn, NullableNumericReadError> {
    let nested_type = data_type
        .strip_prefix("Nullable(")
        .and_then(|inner| inner.strip_suffix(')'))
        .ok_or_else(|| NullableNumericReadError::UnsupportedType(data_type.to_owned()))?;

    if rows > MAX_NULL_MAP_BYTES {
        return Err(NullableNumericReadError::TooManyRows(rows));
    }

    let values =
        read_numeric_column(data, nested_type, rows).map_err(NullableNumericReadError::Nested)?;
    let mut nulls = vec![0u8; rows];
    null_map
        .read_exact(&mut nulls)
        .map_err(NullableNumericReadError::NullMap)?;

    Ok(NullableNumericColumn {
        values,
        null_map: nulls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct OneByte<R>(R);

    impl<R: Read> Read for OneByte<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let len = buffer.len().min(1);
            self.0.read(&mut buffer[..len])
        }
    }

    struct NoRead;

    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("unexpected read")
        }
    }

    #[test]
    fn keeps_nested_values_for_null_rows() {
        let numbers = [11i32, -77, 33];
        let data = numbers
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let column = read_nullable_numeric_column(
            &mut &[0u8, 1, 2][..],
            &mut data.as_slice(),
            "Nullable(Int32)",
            3,
        )
        .unwrap();

        assert_eq!(column.values(), &NumericColumn::Int32(numbers.to_vec()));
        assert_eq!(column.null_map(), &[0, 1, 2]);
        assert_eq!(column.len(), 3);
        assert!(!column.is_empty());
        assert_eq!(column.is_null(0), Some(false));
        assert_eq!(column.is_null(1), Some(true));
        assert_eq!(column.is_null(2), Some(true));
        assert_eq!(column.is_null(3), None);
    }

    #[test]
    fn preserves_float_bits_for_nullable_values() {
        let bits = [0x8000_0000_0000_0000u64, 0x7ff8_0000_0000_0001];
        let data = bits
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let column = read_nullable_numeric_column(
            &mut &[0u8, 1][..],
            &mut data.as_slice(),
            "Nullable(Float64)",
            2,
        )
        .unwrap();

        let NumericColumn::Float64(values) = column.values() else {
            panic!("wrong nested type");
        };
        assert_eq!(values.iter().map(|value| value.to_bits()).collect::<Vec<_>>(), bits);
    }

    #[test]
    fn supports_short_reads_without_consuming_next_batch() {
        let mut nulls = OneByte(Cursor::new([0u8, 1, 99]));
        let mut data = OneByte(Cursor::new([7u8, 0, 8, 0, 0xff]));
        let column =
            read_nullable_numeric_column(&mut nulls, &mut data, "Nullable(UInt16)", 2).unwrap();

        assert_eq!(column.values(), &NumericColumn::UInt16(vec![7, 8]));
        assert_eq!(nulls.0.position(), 2);
        assert_eq!(data.0.position(), 4);
    }

    #[test]
    fn rejects_truncated_null_map() {
        let error = read_nullable_numeric_column(
            &mut &[0u8][..],
            &mut &[4u8, 5][..],
            "Nullable(UInt8)",
            2,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            NullableNumericReadError::NullMap(source)
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn rejects_truncated_nested_values() {
        let error = read_nullable_numeric_column(
            &mut &[0u8, 1][..],
            &mut &[4u8, 5, 6][..],
            "Nullable(UInt16)",
            2,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            NullableNumericReadError::Nested(NumericReadError::Io(source))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn rejects_unsupported_types_before_reading() {
        for data_type in [
            "UInt8",
            "Nullable(String)",
            "Nullable(Nullable(UInt8))",
            "Nullable(UInt8",
        ] {
            assert!(
                read_nullable_numeric_column(&mut NoRead, &mut NoRead, data_type, 2).is_err(),
                "{data_type}"
            );
        }
    }

    #[test]
    fn bounds_both_null_map_and_nested_values_before_reading() {
        assert!(matches!(
            read_nullable_numeric_column(
                &mut NoRead,
                &mut NoRead,
                "Nullable(UInt8)",
                MAX_NULL_MAP_BYTES + 1,
            ),
            Err(NullableNumericReadError::TooManyRows(_))
        ));
        assert!(matches!(
            read_nullable_numeric_column(
                &mut NoRead,
                &mut NoRead,
                "Nullable(UInt64)",
                MAX_NULL_MAP_BYTES / 8 + 1,
            ),
            Err(NullableNumericReadError::Nested(
                NumericReadError::BatchTooLarge { .. }
            ))
        ));
    }

    #[test]
    fn empty_batches_do_not_read_either_stream() {
        let column =
            read_nullable_numeric_column(&mut NoRead, &mut NoRead, "Nullable(Int64)", 0).unwrap();

        assert!(column.is_empty());
        assert_eq!(column.len(), 0);
        assert_eq!(column.values(), &NumericColumn::Int64(Vec::new()));
        assert_eq!(column.is_null(0), None);
    }
}
