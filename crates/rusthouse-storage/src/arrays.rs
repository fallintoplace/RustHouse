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

// Keep small and large mark decoders on the same validation path. The
// const mode lets the compiler remove the encoding branch for either caller.
#[inline]
fn append_array_offset<const ABSOLUTE: bool>(
    base: u64,
    absolute: &mut u64,
    encoded: u64,
    row: usize,
    offsets: &mut Vec<usize>,
) -> Result<(), ArrayReadError> {
    let next = if ABSOLUTE {
        encoded
    } else {
        absolute.checked_add(encoded).ok_or(ArrayReadError::InvalidOffset {
            row,
            reason: "array size overflows u64",
        })?
    };
    if next < *absolute {
        return Err(ArrayReadError::InvalidOffset {
            row,
            reason: "offset moves backwards",
        });
    }
    let elements = next - base;
    if elements > MAX_BATCH_ELEMENTS {
        return Err(ArrayReadError::TooManyElements(elements));
    }
    offsets.push(elements as usize);
    *absolute = next;
    Ok(())
}

// Specialize the two offset encodings before the hot decode loop. The
// compiler can eliminate the per-element mode match for million-row arrays.
fn decode_array_offsets<const ABSOLUTE: bool, O: Read>(
    offset_stream: &mut O,
    rows: usize,
    base: u64,
) -> Result<ArrayOffsets, ArrayReadError> {
    let mut absolute = base;
    let mut offsets = Vec::with_capacity(rows + 1);
    offsets.push(0);

    // Decode small Array marks without clearing the 64 KiB scratch buffer.
    // A complete 64-row offset batch fits in 512 bytes.
    const SMALL_ROWS: usize = 64;
    if rows <= SMALL_ROWS {
        let mut scratch = [0u8; SMALL_ROWS * OFFSET_ENTRY_BYTES];
        let bytes = rows * OFFSET_ENTRY_BYTES;
        offset_stream
            .read_exact(&mut scratch[..bytes])
            .map_err(ArrayReadError::Offsets)?;
        for (row, record) in scratch[..bytes].chunks_exact(OFFSET_ENTRY_BYTES).enumerate() {
            let encoded = u64::from_le_bytes(record.try_into().unwrap());
            append_array_offset::<ABSOLUTE>(base, &mut absolute, encoded, row, &mut offsets)?;
        }
        return Ok(ArrayOffsets { offsets, last_absolute_offset: absolute });
    }

    // Typical medium marks fit within 8 KiB. Decode them with one exact
    // read instead of initializing the 64 KiB bulk buffer.
    const MEDIUM_ROWS: usize = 1024;
    if rows <= MEDIUM_ROWS {
        let mut scratch = [0u8; MEDIUM_ROWS * OFFSET_ENTRY_BYTES];
        let bytes = rows * OFFSET_ENTRY_BYTES;
        offset_stream.read_exact(&mut scratch[..bytes])
            .map_err(ArrayReadError::Offsets)?;
        for (row, record) in scratch[..bytes].chunks_exact(OFFSET_ENTRY_BYTES).enumerate() {
            let encoded = u64::from_le_bytes(record.try_into().unwrap());
            append_array_offset::<ABSOLUTE>(base, &mut absolute, encoded, row, &mut offsets)?;
        }
        return Ok(ArrayOffsets { offsets, last_absolute_offset: absolute });
    }

    let mut buffer = [0u8; 64 * 1024];
    let mut row = 0;
    while row < rows {
        let count = (rows - row).min(buffer.len() / OFFSET_ENTRY_BYTES);
        offset_stream
            .read_exact(&mut buffer[..count * OFFSET_ENTRY_BYTES])
            .map_err(ArrayReadError::Offsets)?;

        for bytes in buffer[..count * OFFSET_ENTRY_BYTES].chunks_exact(OFFSET_ENTRY_BYTES) {
            let encoded = u64::from_le_bytes(bytes.try_into().unwrap());
            append_array_offset::<ABSOLUTE>(base, &mut absolute, encoded, row, &mut offsets)?;
            row += 1;
        }
    }

    Ok(ArrayOffsets {
        offsets,
        last_absolute_offset: absolute,
    })
}

pub fn read_array_offsets<O: Read>(
    offset_stream: &mut O,
    rows: usize,
    encoding: ArrayOffsetsEncoding,
) -> Result<ArrayOffsets, ArrayReadError> {
    if rows > MAX_BATCH_ROWS {
        return Err(ArrayReadError::TooManyRows(rows));
    }
    match encoding {
        ArrayOffsetsEncoding::Sizes => decode_array_offsets::<false, O>(offset_stream, rows, 0),
        ArrayOffsetsEncoding::Absolute { previous_offset } =>
            decode_array_offsets::<true, O>(offset_stream, rows, previous_offset),
    }
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
    fn small_array_marks_keep_absolute_bases_and_input_position() {
        use std::io::Cursor;
        for rows in [0usize, 1, 32, 63, 64, 65] {
            let mut data = (1..=rows as u64).map(|row| 100 + row)
                .flat_map(u64::to_le_bytes).collect::<Vec<_>>();
            data.extend_from_slice(&9999u64.to_le_bytes());
            let mut input = Cursor::new(data);
            let values = read_array_offsets(
                &mut input, rows,
                ArrayOffsetsEncoding::Absolute { previous_offset: 100 },
            ).unwrap();
            assert_eq!(values.len(), rows);
            assert_eq!(values.element_count(), rows);
            assert_eq!(values.last_absolute_offset(), 100 + rows as u64);
            assert_eq!(input.position(), (rows * 8) as u64);
            let mut trailing = [0u8; 8];
            input.read_exact(&mut trailing).unwrap();
            assert_eq!(u64::from_le_bytes(trailing), 9999);
        }
        assert!(matches!(
            read_array_offsets(&mut &[0u8; 7][..], 1, ArrayOffsetsEncoding::Sizes),
            Err(ArrayReadError::Offsets(error)) if error.kind() == io::ErrorKind::UnexpectedEof
        ));
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

#[cfg(test)]
mod specialized_array_offset_mode_tests {
    use super::*;

    #[test]
    fn large_absolute_and_size_modes_produce_identical_relative_offsets() {
        let sizes = (0..24_000u64).map(|index| (index % 7 == 0) as u64 * 3)
            .collect::<Vec<_>>();
        let mut final_offset = 50_000u64;
        let absolute = sizes.iter().map(|&size| {
            final_offset += size;
            final_offset
        }).collect::<Vec<_>>();
        let size_bytes = sizes.iter().flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let absolute_bytes = absolute.iter().flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let decoded_sizes = read_array_offsets(
            &mut size_bytes.as_slice(), sizes.len(), ArrayOffsetsEncoding::Sizes
        ).unwrap();
        let decoded_absolute = read_array_offsets(
            &mut absolute_bytes.as_slice(), absolute.len(),
            ArrayOffsetsEncoding::Absolute { previous_offset: 50_000 }
        ).unwrap();
        assert_eq!(decoded_sizes.offsets(), decoded_absolute.offsets());
        assert_eq!(decoded_absolute.last_absolute_offset(), final_offset);
        assert_eq!(decoded_sizes.element_count(), (final_offset - 50_000) as usize);
    }

    #[test]
    fn specialized_modes_reject_overflow_and_backward_offsets() {
        let sizes = [u64::MAX, 1u64].into_iter().flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>();
        assert!(read_array_offsets(&mut sizes.as_slice(), 2, ArrayOffsetsEncoding::Sizes).is_err());
        let absolute = [5u64].into_iter().flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>();
        assert!(read_array_offsets(
            &mut absolute.as_slice(), 1,
            ArrayOffsetsEncoding::Absolute { previous_offset: 10 },
        ).is_err());
    }
}

#[cfg(test)]
mod small_offsets_codegen_regression_tests {
    use super::*;

    #[test]
    fn small_and_large_batches_use_identical_size_and_absolute_semantics() {
        for rows in [0usize, 1, 2, 63, 64, 65, 128] {
            let sizes = (0..rows).map(|index| (index % 7) as u64).collect::<Vec<_>>();
            let encoded_sizes = sizes.iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>();
            let mut offsets = Vec::with_capacity(rows + 1);
            offsets.push(0);
            let mut absolute = 1000u64;
            let absolute_data = sizes.iter().map(|value| {
                absolute += value;
                offsets.push((absolute - 1000) as usize);
                absolute
            }).flat_map(|value| value.to_le_bytes()).collect::<Vec<_>>();
            let from_sizes = read_array_offsets(
                &mut encoded_sizes.as_slice(), rows, ArrayOffsetsEncoding::Sizes
            ).unwrap();
            let from_absolute = read_array_offsets(
                &mut absolute_data.as_slice(),
                rows,
                ArrayOffsetsEncoding::Absolute { previous_offset: 1000 },
            ).unwrap();
            assert_eq!(from_sizes.offsets(), offsets);
            assert_eq!(from_absolute.offsets(), offsets);
            assert_eq!(from_absolute.last_absolute_offset(), absolute);
        }
    }

    #[test]
    fn small_absolute_rows_still_reject_backward_offsets_and_overflows() {
        let backward_bytes = 6u64.to_le_bytes();
        let mut backward = backward_bytes.as_slice();
        assert!(matches!(
            read_array_offsets(
                &mut backward, 1, ArrayOffsetsEncoding::Absolute { previous_offset: 7 }
            ),
            Err(ArrayReadError::InvalidOffset { reason: "offset moves backwards", .. })
        ));
        let overflow_bytes = 1u64.to_le_bytes();
        let mut overflow = overflow_bytes.as_slice();
        assert!(matches!(
            read_array_offsets(
                &mut overflow, 1, ArrayOffsetsEncoding::Absolute { previous_offset: u64::MAX }
            ),
            Err(ArrayReadError::InvalidOffset { reason: "offset moves backwards", .. })
        ));
    }
}

#[cfg(test)]
mod small_array_mark_regression_tests {
    use super::*;

    fn encoded(values: &[u64]) -> Vec<u8> {
        values.iter().flat_map(|value| value.to_le_bytes()).collect()
    }

    #[test]
    fn small_size_and_absolute_marks_share_exact_boundaries() {
        let sizes = encoded(&[2, 0, 3]);
        let offsets = read_array_offsets(
            &mut sizes.as_slice(),
            3,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();
        assert_eq!(offsets.offsets(), &[0, 2, 2, 5]);
        assert_eq!(offsets.last_absolute_offset(), 5);

        let absolutes = encoded(&[12, 12, 15]);
        let offsets = read_array_offsets(
            &mut absolutes.as_slice(),
            3,
            ArrayOffsetsEncoding::Absolute { previous_offset: 10 },
        )
        .unwrap();
        assert_eq!(offsets.offsets(), &[0, 2, 2, 5]);
        assert_eq!(offsets.last_absolute_offset(), 15);
    }

    #[test]
    fn small_marks_reject_backward_and_oversized_offsets() {
        let backward = encoded(&[9]);
        assert!(matches!(
            read_array_offsets(
                &mut backward.as_slice(),
                1,
                ArrayOffsetsEncoding::Absolute { previous_offset: 10 },
            ),
            Err(ArrayReadError::InvalidOffset { .. })
        ));
        let oversized = encoded(&[MAX_BATCH_ELEMENTS + 1]);
        assert!(matches!(
            read_array_offsets(&mut oversized.as_slice(), 1, ArrayOffsetsEncoding::Sizes),
            Err(ArrayReadError::TooManyElements(_))
        ));
    }
}

#[cfg(test)]
mod medium_array_mark_tests {
    use super::*;
    use std::io::{Cursor, Read};

    #[test]
    fn absolute_and_size_offsets_keep_same_values_across_scratch_thresholds() {
        for rows in [64usize, 65, 512, 1024, 1025, 8192, 8193] {
            for absolute in [false, true] {
                let base = 5_000u64;
                let mut raw = Vec::with_capacity((rows + 1) * 8);
                for i in 0..rows {
                    let value = if absolute { base + i as u64 + 1 } else { 1 };
                    raw.extend_from_slice(&value.to_le_bytes());
                }
                raw.extend_from_slice(&123456789u64.to_le_bytes());
                let mut source = Cursor::new(raw);
                let mode = if absolute {
                    ArrayOffsetsEncoding::Absolute { previous_offset: base }
                } else {
                    ArrayOffsetsEncoding::Sizes
                };
                let decoded = read_array_offsets(&mut source, rows, mode).unwrap();
                assert_eq!(decoded.element_count(), rows, "rows={rows} absolute={absolute}");
                assert_eq!(decoded.last_absolute_offset(), rows as u64 + if absolute { base } else { 0 });
                assert!(decoded.offsets().iter().copied().eq(0..=rows));
                assert_eq!(source.position(), (rows * 8) as u64);
                let mut sentinel = [0u8; 8];
                source.read_exact(&mut sentinel).unwrap();
                assert_eq!(sentinel, 123456789u64.to_le_bytes());
            }
        }
    }

    #[test]
    fn medium_marks_reject_truncation_and_overflow_before_reading_values() {
        for rows in [65usize, 1024] {
            let mut short = vec![0u8; rows * 8 - 1].as_slice();
            assert!(matches!(
                read_array_offsets(&mut short, rows, ArrayOffsetsEncoding::Sizes),
                Err(ArrayReadError::Offsets(error)) if error.kind() == io::ErrorKind::UnexpectedEof
            ));
        }
        let mut overflowing = vec![0u8; 65 * 8];
        overflowing[..8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(
            read_array_offsets(&mut overflowing.as_slice(), 65, ArrayOffsetsEncoding::Sizes),
            Err(ArrayReadError::TooManyElements(_))
        ));
    }
}
