use crate::{NumericColumn, NumericReadError, read_numeric_column};
use std::error::Error;
use std::fmt;
use std::io::{self, Read};
use std::ops::Range;

const MAX_BATCH_ROWS: usize = 1_000_000;
const MAX_BATCH_ELEMENTS: u64 = 64 * 1024 * 1024;
const OFFSET_ENTRY_BYTES: usize = size_of::<u64>();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayOffsetsEncoding {
    Sizes,
    Absolute { previous_offset: u64 },
}

#[derive(Debug, PartialEq, Eq)]
pub struct ArrayOffsets {
    offsets: Vec<usize>,
    last_absolute_offset: u64,
}

impl ArrayOffsets {
    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn offsets(&self) -> &[usize] {
        &self.offsets
    }

    pub fn element_count(&self) -> usize {
        *self.offsets.last().unwrap()
    }

    pub fn last_absolute_offset(&self) -> u64 {
        self.last_absolute_offset
    }

    pub fn row_range(&self, row: usize) -> Option<Range<usize>> {
        let begin = *self.offsets.get(row)?;
        let end = *self.offsets.get(row.checked_add(1)?)?;
        Some(begin..end)
    }
}

#[derive(Debug, PartialEq)]
pub struct NumericArrayColumn {
    offsets: ArrayOffsets,
    values: NumericColumn,
}

impl NumericArrayColumn {
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn offsets(&self) -> &[usize] {
        self.offsets.offsets()
    }

    pub fn values(&self) -> &NumericColumn {
        &self.values
    }

    pub fn row_range(&self, row: usize) -> Option<Range<usize>> {
        self.offsets.row_range(row)
    }
}

#[derive(Debug)]
pub enum ArrayReadError {
    UnsupportedType(String),
    TooManyRows(usize),
    TooManyElements(u64),
    InvalidOffset { row: usize, reason: &'static str },
    Offsets(io::Error),
    Values(NumericReadError),
}

impl fmt::Display for ArrayReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedType(data_type) => {
                write!(f, "expected Array(numeric type), got {data_type}")
            }
            Self::TooManyRows(rows) => write!(f, "Array batch exceeds the row limit: {rows}"),
            Self::TooManyElements(count) => {
                write!(f, "Array batch exceeds the element limit: {count}")
            }
            Self::InvalidOffset { row, reason } => {
                write!(f, "invalid Array offset at row {row}: {reason}")
            }
            Self::Offsets(source) => write!(f, "unable to read Array offsets: {source}"),
            Self::Values(source) => write!(f, "unable to read Array values: {source}"),
        }
    }
}

impl Error for ArrayReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Offsets(source) => Some(source),
            Self::Values(source) => Some(source),
            _ => None,
        }
    }
}

fn next_array_offset(
    encoding: ArrayOffsetsEncoding,
    absolute: u64,
    encoded: u64,
    row: usize,
) -> Result<u64, ArrayReadError> {
    let next = match encoding {
        ArrayOffsetsEncoding::Sizes => {
            absolute
                .checked_add(encoded)
                .ok_or(ArrayReadError::InvalidOffset {
                    row,
                    reason: "array size overflows u64",
                })?
        }
        ArrayOffsetsEncoding::Absolute { .. } => encoded,
    };
    if next < absolute {
        return Err(ArrayReadError::InvalidOffset {
            row,
            reason: "offset moves backwards",
        });
    }
    Ok(next)
}

fn append_array_offset(
    encoding: ArrayOffsetsEncoding,
    base: u64,
    absolute: &mut u64,
    encoded: u64,
    row: usize,
    offsets: &mut Vec<usize>,
) -> Result<(), ArrayReadError> {
    let next = next_array_offset(encoding, *absolute, encoded, row)?;
    let elements = next - base;
    if elements > MAX_BATCH_ELEMENTS {
        return Err(ArrayReadError::TooManyElements(elements));
    }
    offsets.push(elements as usize);
    *absolute = next;
    Ok(())
}

pub fn read_array_offsets<O: Read>(
    offset_stream: &mut O,
    rows: usize,
    encoding: ArrayOffsetsEncoding,
) -> Result<ArrayOffsets, ArrayReadError> {
    if rows > MAX_BATCH_ROWS {
        return Err(ArrayReadError::TooManyRows(rows));
    }

    let base = match encoding {
        ArrayOffsetsEncoding::Sizes => 0,
        ArrayOffsetsEncoding::Absolute { previous_offset } => previous_offset,
    };
    let mut absolute = base;
    let mut offsets = Vec::with_capacity(rows + 1);
    offsets.push(0);

    let mut buffer = [0u8; 64 * 1024];
    let mut row = 0;
    while row < rows {
        let count = (rows - row).min(buffer.len() / OFFSET_ENTRY_BYTES);
        offset_stream
            .read_exact(&mut buffer[..count * OFFSET_ENTRY_BYTES])
            .map_err(ArrayReadError::Offsets)?;

        for bytes in buffer[..count * OFFSET_ENTRY_BYTES].chunks_exact(OFFSET_ENTRY_BYTES) {
            let encoded = u64::from_le_bytes(bytes.try_into().unwrap());
            append_array_offset(encoding, base, &mut absolute, encoded, row, &mut offsets)?;
            row += 1;
        }
    }

    Ok(ArrayOffsets {
        offsets,
        last_absolute_offset: absolute,
    })
}

fn validated_numeric_array_element_type(data_type: &str) -> Result<&str, ArrayReadError> {
    let nested_type = data_type
        .strip_prefix("Array(")
        .and_then(|inner| inner.strip_suffix(')'))
        .filter(|inner| !inner.is_empty())
        .ok_or_else(|| ArrayReadError::UnsupportedType(data_type.to_owned()))?;
    if crate::numeric::numeric_storage_type(nested_type).is_none() {
        return Err(ArrayReadError::UnsupportedType(data_type.to_owned()));
    }
    Ok(nested_type)
}

pub fn read_numeric_array_column<O: Read, D: Read>(
    offset_stream: &mut O,
    data_stream: &mut D,
    data_type: &str,
    rows: usize,
    encoding: ArrayOffsetsEncoding,
) -> Result<NumericArrayColumn, ArrayReadError> {
    let nested_type = validated_numeric_array_element_type(data_type)?;

    let offsets = read_array_offsets(offset_stream, rows, encoding)?;
    let values = read_numeric_column(data_stream, nested_type, offsets.element_count())
        .map_err(ArrayReadError::Values)?;
    Ok(NumericArrayColumn { offsets, values })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
            panic!("unexpected read")
        }
    }

    fn encoded_offsets(values: &[u64]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn reads_8192_offsets_in_one_bulk_read() {
        struct Counting<'a> {
            inner: &'a [u8],
            calls: usize,
        }
        impl Read for Counting<'_> {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.calls += 1;
                self.inner.read(bytes)
            }
        }
        let data = vec![0u8; 8192 * OFFSET_ENTRY_BYTES];
        let mut reader = Counting {
            inner: &data,
            calls: 0,
        };
        let offsets = read_array_offsets(&mut reader, 8192, ArrayOffsetsEncoding::Sizes).unwrap();
        assert_eq!(offsets.len(), 8192);
        assert_eq!(offsets.element_count(), 0);
        assert_eq!(reader.calls, 1);
    }

    #[test]
    fn reads_array_sizes_with_empty_rows_and_short_reads() {
        let mut offsets = OneByte(Cursor::new(encoded_offsets(&[2, 0, 3, 0])));
        let mut values = OneByte(Cursor::new([1u8, 0, 2, 0, 3, 0, 4, 0, 5, 0, 99]));
        let column = read_numeric_array_column(
            &mut offsets,
            &mut values,
            "Array(UInt16)",
            4,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();

        assert_eq!(column.len(), 4);
        assert!(!column.is_empty());
        assert_eq!(column.offsets(), &[0, 2, 2, 5, 5]);
        assert_eq!(column.values(), &NumericColumn::UInt16(vec![1, 2, 3, 4, 5]));
        assert_eq!(column.row_range(0), Some(0..2));
        assert_eq!(column.row_range(1), Some(2..2));
        assert_eq!(column.row_range(2), Some(2..5));
        assert_eq!(column.row_range(3), Some(5..5));
        assert_eq!(column.row_range(4), None);
        assert_eq!(column.row_range(usize::MAX), None);
        assert_eq!(offsets.0.position(), 32);
        assert_eq!(values.0.position(), 10);
    }

    #[test]
    fn reads_absolute_offsets_relative_to_previous_batch() {
        let mut offsets = Cursor::new(encoded_offsets(&[12, 12, 15, 99]));
        let mut values = Cursor::new([3u8, 4, 5, 6, 7, 99]);
        let column = read_numeric_array_column(
            &mut offsets,
            &mut values,
            "Array(Int8)",
            3,
            ArrayOffsetsEncoding::Absolute {
                previous_offset: 10,
            },
        )
        .unwrap();

        assert_eq!(column.offsets(), &[0, 2, 2, 5]);
        assert_eq!(column.values(), &NumericColumn::Int8(vec![3, 4, 5, 6, 7]));
        assert_eq!(offsets.position(), 24);
        assert_eq!(values.position(), 5);
    }

    #[test]
    fn decodes_float_bits_without_conversion() {
        let bits = [0x8000_0000u32, 0x7fc0_1234];
        let values = bits
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let mut data = values.as_slice();
        let column = read_numeric_array_column(
            &mut encoded_offsets(&[2]).as_slice(),
            &mut data,
            "Array(Float32)",
            1,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();

        let NumericColumn::Float32(values) = column.values() else {
            panic!("wrong nested type");
        };
        assert_eq!(
            values
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            bits
        );
    }

    #[test]
    fn rejects_decreasing_absolute_offsets_before_reading_values() {
        let encoded = encoded_offsets(&[3, 2]);
        let mut offsets = encoded.as_slice();
        assert!(matches!(
            read_numeric_array_column(
                &mut offsets,
                &mut NoRead,
                "Array(UInt8)",
                2,
                ArrayOffsetsEncoding::Absolute { previous_offset: 0 },
            ),
            Err(ArrayReadError::InvalidOffset {
                row: 1,
                reason: "offset moves backwards"
            })
        ));
    }

    #[test]
    fn rejects_oversized_and_overflowing_arrays_without_reading_values() {
        let encoded = encoded_offsets(&[MAX_BATCH_ELEMENTS + 1]);
        let mut offsets = encoded.as_slice();
        assert!(matches!(
            read_numeric_array_column(
                &mut offsets,
                &mut NoRead,
                "Array(UInt8)",
                1,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(ArrayReadError::TooManyElements(_))
        ));

        let encoded = encoded_offsets(&[1]);
        let mut offsets = encoded.as_slice();
        assert!(matches!(
            read_numeric_array_column(
                &mut offsets,
                &mut NoRead,
                "Array(UInt8)",
                1,
                ArrayOffsetsEncoding::Absolute {
                    previous_offset: u64::MAX,
                },
            ),
            Err(ArrayReadError::InvalidOffset { row: 0, .. })
        ));
    }

    #[test]
    fn rejects_truncated_offset_and_value_streams() {
        assert!(matches!(
            read_numeric_array_column(
                &mut &[0u8; 7][..],
                &mut NoRead,
                "Array(UInt8)",
                1,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(ArrayReadError::Offsets(source))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));
        assert!(matches!(
            read_numeric_array_column(
                &mut encoded_offsets(&[2]).as_slice(),
                &mut &[42u8][..],
                "Array(UInt8)",
                1,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(ArrayReadError::Values(NumericReadError::Io(source)))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn rejects_unsupported_types_and_row_counts_without_reading() {
        for data_type in ["UInt8", "Array()", "Array(UInt8", "Array(UInt8) "] {
            assert!(matches!(
                read_numeric_array_column(
                    &mut NoRead,
                    &mut NoRead,
                    data_type,
                    1,
                    ArrayOffsetsEncoding::Sizes,
                ),
                Err(ArrayReadError::UnsupportedType(_))
            ));
        }
        assert!(matches!(
            read_numeric_array_column(
                &mut NoRead,
                &mut NoRead,
                "Array(UInt8)",
                MAX_BATCH_ROWS + 1,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(ArrayReadError::TooManyRows(_))
        ));

        assert!(matches!(
            read_numeric_array_column(
                &mut &[0u8; 8][..],
                &mut NoRead,
                "Array(String)",
                1,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(ArrayReadError::UnsupportedType(_))
        ));
    }

    #[test]
    fn rejects_unsupported_element_types_before_reading_offsets() {
        struct NoRead;

        impl Read for NoRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("invalid type must fail before reading")
            }
        }

        assert!(matches!(
            read_numeric_array_column(
                &mut NoRead,
                &mut NoRead,
                "Array(String)",
                1,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(ArrayReadError::UnsupportedType(_))
        ));
    }

    #[test]
    fn empty_batches_leave_streams_untouched() {
        let column = read_numeric_array_column(
            &mut NoRead,
            &mut NoRead,
            "Array(UInt64)",
            0,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();
        assert!(column.is_empty());
        assert_eq!(column.len(), 0);
        assert_eq!(column.offsets(), &[0]);
        assert_eq!(column.values(), &NumericColumn::UInt64(Vec::new()));
    }
    #[test]
    fn reads_offsets_without_a_numeric_value_stream() {
        let sizes = encoded_offsets(&[2, 0, 3]);
        let mut sizes = OneByte(Cursor::new(sizes));
        let offsets = read_array_offsets(&mut sizes, 3, ArrayOffsetsEncoding::Sizes).unwrap();
        assert_eq!(offsets.len(), 3);
        assert!(!offsets.is_empty());
        assert_eq!(offsets.element_count(), 5);
        assert_eq!(offsets.last_absolute_offset(), 5);
        assert_eq!(offsets.offsets(), &[0, 2, 2, 5]);
        assert_eq!(offsets.row_range(0), Some(0..2));
        assert_eq!(offsets.row_range(1), Some(2..2));
        assert_eq!(offsets.row_range(2), Some(2..5));
        assert_eq!(offsets.row_range(usize::MAX), None);

        let absolute = encoded_offsets(&[12, 12, 15]);
        let offsets = read_array_offsets(
            &mut absolute.as_slice(),
            3,
            ArrayOffsetsEncoding::Absolute {
                previous_offset: 10,
            },
        )
        .unwrap();
        assert_eq!(offsets.offsets(), &[0, 2, 2, 5]);
        assert_eq!(offsets.last_absolute_offset(), 15);
    }

    #[test]
    fn reads_consecutive_offset_batches_with_a_running_base() {
        let encoded = encoded_offsets(&[2, 5, 5, 9]);
        let mut reader = encoded.as_slice();
        let first = read_array_offsets(
            &mut reader,
            2,
            ArrayOffsetsEncoding::Absolute { previous_offset: 0 },
        )
        .unwrap();
        let second = read_array_offsets(
            &mut reader,
            2,
            ArrayOffsetsEncoding::Absolute {
                previous_offset: first.last_absolute_offset(),
            },
        )
        .unwrap();
        assert_eq!(first.offsets(), &[0, 2, 5]);
        assert_eq!(second.offsets(), &[0, 0, 4]);
        assert!(reader.is_empty());
    }
}

#[cfg(test)]
mod numeric_array_type_fastcheck_tests {
    use super::*;

    #[test]
    fn validates_numeric_array_inner_type_without_decoding() {
        for ty in ["Array(UInt64)", "Array(Int16)", "Array(Float64)", "Array(Date)",
            "Array(DateTime64(3, 'UTC'))"] {
            assert!(validated_numeric_array_element_type(ty).is_ok(), "{ty}");
        }
        for ty in ["Array(String)", "Array(Nullable(UInt64))", "Array(garbage)", "Array()"] {
            assert!(validated_numeric_array_element_type(ty).is_err(), "{ty}");
        }
    }
}
