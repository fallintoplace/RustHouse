use crate::{
    ArrayOffsets, ArrayOffsetsEncoding, ArrayReadError, NumericColumn, NumericReadError,
    read_array_offsets, read_numeric_column,
};
use std::error::Error;
use std::fmt;
use std::io::{self, Read};
use std::ops::Range;

/// Flattened Array(Array(T)) with two levels of offsets.
/// Outer offsets address inner arrays; inner offsets address scalar elements.
#[derive(Debug, PartialEq)]
pub struct NestedNumericArrayColumn {
    outer: ArrayOffsets,
    inner: ArrayOffsets,
    values: NumericColumn,
}

impl NestedNumericArrayColumn {
    pub fn len(&self) -> usize {
        self.outer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.outer.is_empty()
    }

    pub fn outer_offsets(&self) -> &[usize] {
        self.outer.offsets()
    }

    pub fn inner_offsets(&self) -> &[usize] {
        self.inner.offsets()
    }

    pub fn row_range(&self, row: usize) -> Option<Range<usize>> {
        self.outer.row_range(row)
    }

    pub fn inner_array_range(&self, array: usize) -> Option<Range<usize>> {
        self.inner.row_range(array)
    }

    pub fn last_outer_absolute_offset(&self) -> u64 {
        self.outer.last_absolute_offset()
    }

    pub fn last_inner_absolute_offset(&self) -> u64 {
        self.inner.last_absolute_offset()
    }

    pub fn values(&self) -> &NumericColumn {
        &self.values
    }
}

#[derive(Debug)]
pub enum NestedArrayReadError {
    InvalidType(String),
    OuterOffsets(ArrayReadError),
    InnerOffsets(ArrayReadError),
    Values(NumericReadError),
}

impl fmt::Display for NestedArrayReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidType(data_type) => {
                write!(f, "expected Array(Array(numeric type)), got {data_type}")
            }
            Self::OuterOffsets(source) => write!(f, "outer Array offsets: {source}"),
            Self::InnerOffsets(source) => write!(f, "inner Array offsets: {source}"),
            Self::Values(source) => write!(f, "nested Array values: {source}"),
        }
    }
}

impl Error for NestedArrayReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidType(_) => None,
            Self::OuterOffsets(source) | Self::InnerOffsets(source) => Some(source),
            Self::Values(source) => Some(source),
        }
    }
}

fn validated_nested_numeric_leaf_type(data_type: &str) -> Result<&str, NestedArrayReadError> {
    let leaf_type = data_type
        .strip_prefix("Array(Array(")
        .and_then(|inner| inner.strip_suffix("))"))
        .filter(|inner| !inner.is_empty())
        .ok_or_else(|| NestedArrayReadError::InvalidType(data_type.to_owned()))?;
    if read_numeric_column(&mut io::empty(), leaf_type, 0).is_err() {
        return Err(NestedArrayReadError::InvalidType(data_type.to_owned()));
    }
    Ok(leaf_type)
}

/// Decode separate outer and inner offsets and one flattened numeric value stream.
///
/// Each offset stream has its own encoding and absolute position, allowing
/// decoding consecutive granules without resetting cumulative offsets.
pub fn read_nested_numeric_array_column<O: Read, I: Read, D: Read>(
    outer_stream: &mut O,
    inner_stream: &mut I,
    data_stream: &mut D,
    data_type: &str,
    rows: usize,
    outer_encoding: ArrayOffsetsEncoding,
    inner_encoding: ArrayOffsetsEncoding,
) -> Result<NestedNumericArrayColumn, NestedArrayReadError> {
    // Reject invalid leaf types before consuming either offset stream.
    let leaf_type = validated_nested_numeric_leaf_type(data_type)?;
    let outer = read_array_offsets(outer_stream, rows, outer_encoding)
        .map_err(NestedArrayReadError::OuterOffsets)?;
    let inner = read_array_offsets(inner_stream, outer.element_count(), inner_encoding)
        .map_err(NestedArrayReadError::InnerOffsets)?;
    let values = read_numeric_column(data_stream, leaf_type, inner.element_count())
        .map_err(NestedArrayReadError::Values)?;

    Ok(NestedNumericArrayColumn {
        outer,
        inner,
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    struct OneByte<R>(R);

    impl<R: Read> Read for OneByte<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(1);
            self.0.read(&mut buffer[..count])
        }
    }

    struct NoRead;

    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("invalid input must fail before this stream is read")
        }
    }

    fn offsets(values: &[u64]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn decodes_nested_arrays_with_empty_rows_and_empty_inner_arrays() {
        let mut outer = OneByte(Cursor::new(offsets(&[2, 0, 1, 99])));
        let mut inner = OneByte(Cursor::new(offsets(&[2, 0, 3, 99])));
        let encoded = [7u16, 8, 9, 10, 11]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let mut data = OneByte(Cursor::new(encoded));

        let batch = read_nested_numeric_array_column(
            &mut outer,
            &mut inner,
            &mut data,
            "Array(Array(UInt16))",
            3,
            ArrayOffsetsEncoding::Sizes,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();

        assert_eq!(batch.len(), 3);
        assert!(!batch.is_empty());
        assert_eq!(batch.outer_offsets(), &[0, 2, 2, 3]);
        assert_eq!(batch.inner_offsets(), &[0, 2, 2, 5]);
        assert_eq!(batch.row_range(0), Some(0..2));
        assert_eq!(batch.row_range(1), Some(2..2));
        assert_eq!(batch.row_range(2), Some(2..3));
        assert_eq!(batch.row_range(3), None);
        assert_eq!(batch.inner_array_range(0), Some(0..2));
        assert_eq!(batch.inner_array_range(1), Some(2..2));
        assert_eq!(batch.inner_array_range(2), Some(2..5));
        assert_eq!(batch.inner_array_range(3), None);
        assert_eq!(
            batch.values(),
            &NumericColumn::UInt16(vec![7, 8, 9, 10, 11])
        );
        assert_eq!(batch.last_outer_absolute_offset(), 3);
        assert_eq!(batch.last_inner_absolute_offset(), 5);
        assert_eq!(outer.0.position(), 24);
        assert_eq!(inner.0.position(), 24);
        assert_eq!(data.0.position(), 10);
    }

    #[test]
    fn handles_absolute_offsets_from_previous_granules() {
        let encoded = [-5i32, -4, 0, 7, 8]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect::<Vec<_>>();
        let batch = read_nested_numeric_array_column(
            &mut offsets(&[12, 13]).as_slice(),
            &mut offsets(&[52, 52, 55]).as_slice(),
            &mut encoded.as_slice(),
            "Array(Array(Int32))",
            2,
            ArrayOffsetsEncoding::Absolute {
                previous_offset: 10,
            },
            ArrayOffsetsEncoding::Absolute {
                previous_offset: 50,
            },
        )
        .unwrap();

        assert_eq!(batch.outer_offsets(), &[0, 2, 3]);
        assert_eq!(batch.inner_offsets(), &[0, 2, 2, 5]);
        assert_eq!(batch.last_outer_absolute_offset(), 13);
        assert_eq!(batch.last_inner_absolute_offset(), 55);
        assert_eq!(batch.values(), &NumericColumn::Int32(vec![-5, -4, 0, 7, 8]));
    }

    #[test]
    fn rejects_invalid_types_before_reading_offsets() {
        for data_type in [
            "Array(UInt8)",
            "Array(Array())",
            "Array(Array(String))",
            "Array(Array(Nullable(UInt8)))",
            "Array(Array(UInt16)",
            "Array(Array(UInt8)))",
            "Nullable(Array(Array(Int8)))",
        ] {
            assert!(
                matches!(
                    read_nested_numeric_array_column(
                        &mut NoRead,
                        &mut NoRead,
                        &mut NoRead,
                        data_type,
                        1,
                        ArrayOffsetsEncoding::Sizes,
                        ArrayOffsetsEncoding::Sizes,
                    ),
                    Err(NestedArrayReadError::InvalidType(_))
                ),
                "{data_type}"
            );
        }
    }

    #[test]
    fn detects_invalid_outer_and_inner_offsets() {
        assert!(matches!(
            read_nested_numeric_array_column(
                &mut &[0u8; 7][..],
                &mut NoRead,
                &mut NoRead,
                "Array(Array(UInt8))",
                1,
                ArrayOffsetsEncoding::Sizes,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(NestedArrayReadError::OuterOffsets(ArrayReadError::Offsets(error)))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));

        assert!(matches!(
            read_nested_numeric_array_column(
                &mut offsets(&[2]).as_slice(),
                &mut &[0u8; 8][..],
                &mut NoRead,
                "Array(Array(UInt8))",
                1,
                ArrayOffsetsEncoding::Sizes,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(NestedArrayReadError::InnerOffsets(ArrayReadError::Offsets(error)))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));

        assert!(matches!(
            read_nested_numeric_array_column(
                &mut offsets(&[2]).as_slice(),
                &mut offsets(&[5, 4]).as_slice(),
                &mut NoRead,
                "Array(Array(UInt8))",
                1,
                ArrayOffsetsEncoding::Sizes,
                ArrayOffsetsEncoding::Absolute { previous_offset: 0 },
            ),
            Err(NestedArrayReadError::InnerOffsets(
                ArrayReadError::InvalidOffset {
                    row: 1,
                    reason: "offset moves backwards"
                }
            ))
        ));
    }

    #[test]
    fn rejects_too_many_inner_arrays_before_reading_their_offsets() {
        assert!(matches!(
            read_nested_numeric_array_column(
                &mut offsets(&[1_000_001]).as_slice(),
                &mut NoRead,
                &mut NoRead,
                "Array(Array(Int8))",
                1,
                ArrayOffsetsEncoding::Sizes,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(NestedArrayReadError::InnerOffsets(
                ArrayReadError::TooManyRows(1_000_001)
            ))
        ));
    }

    #[test]
    fn catches_truncated_scalar_values() {
        assert!(matches!(
            read_nested_numeric_array_column(
                &mut offsets(&[1]).as_slice(),
                &mut offsets(&[2]).as_slice(),
                &mut &[99u8][..],
                "Array(Array(UInt16))",
                1,
                ArrayOffsetsEncoding::Sizes,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(NestedArrayReadError::Values(NumericReadError::Io(error)))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn zero_rows_do_not_consume_any_stream() {
        let batch = read_nested_numeric_array_column(
            &mut NoRead,
            &mut NoRead,
            &mut NoRead,
            "Array(Array(Float64))",
            0,
            ArrayOffsetsEncoding::Sizes,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();
        assert!(batch.is_empty());
        assert_eq!(batch.outer_offsets(), &[0]);
        assert_eq!(batch.inner_offsets(), &[0]);
        assert_eq!(batch.values(), &NumericColumn::Float64(vec![]));
    }
}
