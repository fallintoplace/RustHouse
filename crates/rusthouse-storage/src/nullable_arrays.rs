use crate::{
    ArrayOffsets, ArrayOffsetsEncoding, ArrayReadError, NullableColumn, NullableReadError,
    NumericColumn, read_array_offsets, read_nullable_numeric_column, read_numeric_column,
};
use std::error::Error;
use std::fmt;
use std::io::{self, Read};
use std::ops::Range;

/// A batch of Array(Nullable(T)) values, where T is a supported numeric type.
#[derive(Debug, PartialEq)]
pub struct NullableNumericArrayColumn {
    offsets: ArrayOffsets,
    values: NullableColumn<NumericColumn>,
}

impl NullableNumericArrayColumn {
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn offsets(&self) -> &[usize] {
        self.offsets.offsets()
    }

    pub fn last_absolute_offset(&self) -> u64 {
        self.offsets.last_absolute_offset()
    }

    pub fn row_range(&self, row: usize) -> Option<Range<usize>> {
        self.offsets.row_range(row)
    }

    pub fn values(&self) -> &NullableColumn<NumericColumn> {
        &self.values
    }
}

#[derive(Debug)]
pub enum NullableArrayReadError {
    InvalidType(String),
    Offsets(ArrayReadError),
    Elements(NullableReadError),
}

impl fmt::Display for NullableArrayReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidType(data_type) => {
                write!(f, "expected Array(Nullable(numeric type)), got {data_type}")
            }
            Self::Offsets(error) => write!(f, "Array offsets: {error}"),
            Self::Elements(error) => write!(f, "Array Nullable values: {error}"),
        }
    }
}

impl Error for NullableArrayReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidType(_) => None,
            Self::Offsets(error) => Some(error),
            Self::Elements(error) => Some(error),
        }
    }
}

fn validated_nullable_array_type(data_type: &str) -> Result<&str, NullableArrayReadError> {
    let nullable_type = data_type
        .strip_prefix("Array(")
        .and_then(|inner| inner.strip_suffix(')'))
        .ok_or_else(|| NullableArrayReadError::InvalidType(data_type.to_owned()))?;
    let numeric_type = nullable_type
        .strip_prefix("Nullable(")
        .and_then(|inner| inner.strip_suffix(')'))
        .filter(|inner| !inner.is_empty())
        .ok_or_else(|| NullableArrayReadError::InvalidType(data_type.to_owned()))?;

    // Validate before consuming any stream. The zero-row scalar read cannot read bytes.
    if read_numeric_column(&mut io::empty(), numeric_type, 0).is_err() {
        return Err(NullableArrayReadError::InvalidType(data_type.to_owned()));
    }

    Ok(nullable_type)
}

/// Decode separate Array offsets, element null-map, and element value streams.
/// All element slots, including null placeholders, consume a physical value.
pub fn read_nullable_numeric_array_column<O: Read, N: Read, D: Read>(
    offsets: &mut O,
    nulls: &mut N,
    data: &mut D,
    data_type: &str,
    rows: usize,
    encoding: ArrayOffsetsEncoding,
) -> Result<NullableNumericArrayColumn, NullableArrayReadError> {
    let nullable_type = validated_nullable_array_type(data_type)?;

    let offsets =
        read_array_offsets(offsets, rows, encoding).map_err(NullableArrayReadError::Offsets)?;
    let values = read_nullable_numeric_column(nulls, data, nullable_type, offsets.element_count())
        .map_err(NullableArrayReadError::Elements)?;

    Ok(NullableNumericArrayColumn { offsets, values })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NumericReadError;
    use std::io::Cursor;

    struct NoRead;

    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("unexpected read")
        }
    }

    fn offsets(values: &[u64]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn decodes_array_nullable_values_and_empty_rows() {
        let mut encoded_offsets = Cursor::new(offsets(&[2, 0, 1, 99]));
        let mut nulls = Cursor::new([0, 1, 0, 99]);
        let mut values = Cursor::new(
            [10u16, 0, 30, 99]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );

        let batch = read_nullable_numeric_array_column(
            &mut encoded_offsets,
            &mut nulls,
            &mut values,
            "Array(Nullable(UInt16))",
            3,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();
        assert_eq!(batch.len(), 3);
        assert!(!batch.is_empty());
        assert_eq!(batch.offsets(), &[0, 2, 2, 3]);
        assert_eq!(batch.row_range(0), Some(0..2));
        assert_eq!(batch.row_range(1), Some(2..2));
        assert_eq!(batch.row_range(2), Some(2..3));
        assert_eq!(batch.row_range(3), None);
        assert_eq!(batch.last_absolute_offset(), 3);
        assert_eq!(batch.values().null_map(), &[0, 1, 0]);
        assert_eq!(batch.values().is_null(1), Some(true));
        assert_eq!(
            batch.values().values(),
            &NumericColumn::UInt16(vec![10, 0, 30])
        );
        assert_eq!(encoded_offsets.position(), 24);
        assert_eq!(nulls.position(), 3);
        assert_eq!(values.position(), 6);
    }

    #[test]
    fn supports_absolute_offsets_across_batches() {
        let encoded = offsets(&[7, 7, 9]);
        let mut nulls = &[1u8, 0, 0, 1][..];
        let data = [0i32, -4, 9, 0]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect::<Vec<_>>();
        let batch = read_nullable_numeric_array_column(
            &mut encoded.as_slice(),
            &mut nulls,
            &mut data.as_slice(),
            "Array(Nullable(Int32))",
            3,
            ArrayOffsetsEncoding::Absolute { previous_offset: 5 },
        )
        .unwrap();
        assert_eq!(batch.offsets(), &[0, 2, 2, 4]);
        assert_eq!(batch.last_absolute_offset(), 9);
        assert_eq!(batch.values().null_map(), &[1, 0, 0, 1]);
        assert_eq!(
            batch.values().values(),
            &NumericColumn::Int32(vec![0, -4, 9, 0])
        );
    }

    #[test]
    fn rejects_invalid_types_before_reading_streams() {
        for ty in [
            "Array(UInt8)",
            "Nullable(Array(UInt8))",
            "Array(Nullable())",
            "Array(Nullable(Float16))",
            "Array(Nullable(UInt8)",
        ] {
            assert!(
                matches!(
                    read_nullable_numeric_array_column(
                        &mut NoRead,
                        &mut NoRead,
                        &mut NoRead,
                        ty,
                        1,
                        ArrayOffsetsEncoding::Sizes,
                    ),
                    Err(NullableArrayReadError::InvalidType(_))
                ),
                "{ty}"
            );
        }
    }

    #[test]
    fn rejects_truncated_null_map_before_reading_values() {
        let encoded = offsets(&[2]);
        let err = read_nullable_numeric_array_column(
            &mut encoded.as_slice(),
            &mut &[0u8][..],
            &mut NoRead,
            "Array(Nullable(UInt8))",
            1,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            NullableArrayReadError::Elements(NullableReadError::Io(source))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn rejects_truncated_values_and_large_element_counts() {
        let encoded = offsets(&[2]);
        let err = read_nullable_numeric_array_column(
            &mut encoded.as_slice(),
            &mut &[0, 1][..],
            &mut &[7u8][..],
            "Array(Nullable(UInt16))",
            1,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            NullableArrayReadError::Elements(NullableReadError::Numeric(NumericReadError::Io(source)))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));

        let encoded = offsets(&[1_000_001]);
        assert!(matches!(
            read_nullable_numeric_array_column(
                &mut encoded.as_slice(),
                &mut NoRead,
                &mut NoRead,
                "Array(Nullable(UInt8))",
                1,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(NullableArrayReadError::Elements(
                NullableReadError::TooManyRows(1_000_001)
            ))
        ));
    }

    #[test]
    fn empty_array_batch_does_not_read_streams() {
        let batch = read_nullable_numeric_array_column(
            &mut NoRead,
            &mut NoRead,
            &mut NoRead,
            "Array(Nullable(Float64))",
            0,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();
        assert!(batch.is_empty());
        assert_eq!(batch.offsets(), &[0]);
        assert!(batch.values().is_empty());
    }
}
