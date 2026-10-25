use crate::{NumericColumn, NumericReadError, read_numeric_column};
use std::error::Error;
use std::fmt;
use std::io::Read;
use std::ops::Range;

const MAX_BATCH_ROWS: usize = 1_000_000;
const MAX_BATCH_ELEMENTS: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayOffsetEncoding {
    Absolute,
    RowSizes,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ArrayOffsets {
    start: u64,
    offsets: Vec<u64>,
}

impl ArrayOffsets {
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn offsets(&self) -> &[u64] {
        &self.offsets
    }

    pub fn element_count(&self) -> u64 {
        self.offsets.last().copied().unwrap_or(self.start) - self.start
    }

    pub fn range(&self, row: usize) -> Option<Range<u64>> {
        let end = *self.offsets.get(row)?;
        let start = if row == 0 {
            self.start
        } else {
            self.offsets[row - 1]
        };
        Some(start..end)
    }
}

#[derive(Debug)]
pub enum ArrayOffsetReadError {
    TooManyRows(usize),
    TooManyElements {
        row: usize,
        total: u64,
    },
    DecreasingOffset {
        row: usize,
        previous: u64,
        current: u64,
    },
    Overflow {
        row: usize,
    },
    Numeric(NumericReadError),
}

impl fmt::Display for ArrayOffsetReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRows(rows) => {
                write!(f, "Array offset batch exceeds the row limit: {rows}")
            }
            Self::TooManyElements { row, total } => {
                write!(
                    f,
                    "Array offset batch exceeds the element limit at row {row}: {total}"
                )
            }
            Self::DecreasingOffset {
                row,
                previous,
                current,
            } => write!(
                f,
                "Array offsets decrease at row {row}: {previous} to {current}"
            ),
            Self::Overflow { row } => write!(f, "Array offset overflows at row {row}"),
            Self::Numeric(error) => write!(f, "unable to read Array offsets: {error}"),
        }
    }
}

impl Error for ArrayOffsetReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Numeric(error) => Some(error),
            _ => None,
        }
    }
}

pub fn read_array_offsets<R: Read>(
    reader: &mut R,
    rows: usize,
    encoding: ArrayOffsetEncoding,
    start: u64,
) -> Result<ArrayOffsets, ArrayOffsetReadError> {
    if rows > MAX_BATCH_ROWS {
        return Err(ArrayOffsetReadError::TooManyRows(rows));
    }

    let NumericColumn::UInt64(mut offsets) =
        read_numeric_column(reader, "UInt64", rows).map_err(ArrayOffsetReadError::Numeric)?
    else {
        unreachable!()
    };

    let mut previous = start;
    for (row, offset) in offsets.iter_mut().enumerate() {
        let current = match encoding {
            ArrayOffsetEncoding::Absolute => *offset,
            ArrayOffsetEncoding::RowSizes => previous
                .checked_add(*offset)
                .ok_or(ArrayOffsetReadError::Overflow { row })?,
        };

        if current < previous {
            return Err(ArrayOffsetReadError::DecreasingOffset {
                row,
                previous,
                current,
            });
        }
        let total = current - start;
        if total > MAX_BATCH_ELEMENTS {
            return Err(ArrayOffsetReadError::TooManyElements { row, total });
        }

        *offset = current;
        previous = current;
    }

    Ok(ArrayOffsets { start, offsets })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    fn encode(values: &[u64]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

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
            panic!("unexpected read")
        }
    }

    #[test]
    fn decodes_row_sizes_as_cumulative_offsets() {
        let mut bytes = encode(&[2, 0, 3]);
        bytes.push(99);
        let mut reader = Bytewise(Cursor::new(bytes));

        let result = read_array_offsets(&mut reader, 3, ArrayOffsetEncoding::RowSizes, 0).unwrap();

        assert_eq!(result.len(), 3);
        assert!(!result.is_empty());
        assert_eq!(result.offsets(), &[2, 2, 5]);
        assert_eq!(result.element_count(), 5);
        assert_eq!(result.range(0), Some(0..2));
        assert_eq!(result.range(1), Some(2..2));
        assert_eq!(result.range(2), Some(2..5));
        assert_eq!(result.range(3), None);
        assert_eq!(result.range(usize::MAX), None);
        assert_eq!(reader.0.position(), 24);
    }

    #[test]
    fn decodes_legacy_absolute_offsets() {
        let mut reader = Cursor::new(encode(&[2, 2, 5]));
        let result = read_array_offsets(&mut reader, 3, ArrayOffsetEncoding::Absolute, 0).unwrap();
        assert_eq!(result.offsets(), &[2, 2, 5]);
        assert_eq!(result.element_count(), 5);
        assert_eq!(reader.position(), 24);
    }

    #[test]
    fn carries_offsets_across_batches_in_both_encodings() {
        for (encoding, values) in [
            (ArrayOffsetEncoding::Absolute, vec![7, 7, 10]),
            (ArrayOffsetEncoding::RowSizes, vec![2, 0, 3]),
        ] {
            let result =
                read_array_offsets(&mut encode(&values).as_slice(), 3, encoding, 5).unwrap();
            assert_eq!(result.offsets(), &[7, 7, 10]);
            assert_eq!(result.element_count(), 5);
            assert_eq!(result.range(0), Some(5..7));
            assert_eq!(result.range(1), Some(7..7));
            assert_eq!(result.range(2), Some(7..10));
        }
    }

    #[test]
    fn rejects_decreasing_absolute_offsets_including_batch_boundary() {
        for (start, values, row, previous, current) in
            [(0, vec![2, 1], 1, 2, 1), (5, vec![4], 0, 5, 4)]
        {
            assert!(matches!(
                read_array_offsets(
                    &mut encode(&values).as_slice(),
                    values.len(),
                    ArrayOffsetEncoding::Absolute,
                    start,
                ),
                Err(ArrayOffsetReadError::DecreasingOffset {
                    row: actual_row,
                    previous: actual_previous,
                    current: actual_current,
                }) if actual_row == row && actual_previous == previous && actual_current == current
            ));
        }
    }

    #[test]
    fn rejects_cumulative_overflow_and_excessive_elements() {
        assert!(matches!(
            read_array_offsets(
                &mut encode(&[2]).as_slice(),
                1,
                ArrayOffsetEncoding::RowSizes,
                u64::MAX - 1,
            ),
            Err(ArrayOffsetReadError::Overflow { row: 0 })
        ));
        assert!(matches!(
            read_array_offsets(
                &mut encode(&[MAX_BATCH_ELEMENTS + 1]).as_slice(),
                1,
                ArrayOffsetEncoding::RowSizes,
                0,
            ),
            Err(ArrayOffsetReadError::TooManyElements { row: 0, .. })
        ));
        assert!(matches!(
            read_array_offsets(
                &mut encode(&[MAX_BATCH_ELEMENTS, 1]).as_slice(),
                2,
                ArrayOffsetEncoding::RowSizes,
                0,
            ),
            Err(ArrayOffsetReadError::TooManyElements { row: 1, .. })
        ));
        assert!(matches!(
            read_array_offsets(
                &mut encode(&[MAX_BATCH_ELEMENTS + 1]).as_slice(),
                1,
                ArrayOffsetEncoding::Absolute,
                0,
            ),
            Err(ArrayOffsetReadError::TooManyElements { row: 0, .. })
        ));
    }

    #[test]
    fn rejects_truncated_offsets_and_oversized_batches() {
        let mut bytes = encode(&[2]);
        bytes.extend_from_slice(&[1, 2, 3]);
        assert!(matches!(
            read_array_offsets(
                &mut bytes.as_slice(),
                2,
                ArrayOffsetEncoding::Absolute,
                0,
            ),
            Err(ArrayOffsetReadError::Numeric(NumericReadError::Io(error)))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));
        assert!(matches!(
            read_array_offsets(
                &mut NoRead,
                MAX_BATCH_ROWS + 1,
                ArrayOffsetEncoding::RowSizes,
                0,
            ),
            Err(ArrayOffsetReadError::TooManyRows(_))
        ));
    }

    #[test]
    fn empty_batch_does_not_read_or_move_the_start() {
        let mut input = Cursor::new([42u8]);
        let result = read_array_offsets(&mut input, 0, ArrayOffsetEncoding::RowSizes, 123).unwrap();
        assert!(result.is_empty());
        assert_eq!(result.offsets(), &[]);
        assert_eq!(result.element_count(), 0);
        assert_eq!(result.range(0), None);
        assert_eq!(input.position(), 0);
    }
}
