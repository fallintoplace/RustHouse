mod readers;
pub use self::readers::{read_legacy_strings, read_sized_strings};
pub(crate) use self::readers::read_legacy_strings_buffered;
use self::readers::*;

use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, Read};

const MAX_BATCH_ROWS: usize = 1_000_000;
const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;
const SIZE_ENTRY_BYTES: usize = size_of::<u64>();

#[derive(Debug, PartialEq, Eq)]
pub struct StringColumn {
    data: Vec<u8>,
    offsets: Vec<usize>,
}

impl StringColumn {
    fn new(rows: usize) -> Self {
        let mut offsets = Vec::with_capacity(rows + 1);
        offsets.push(0);
        Self {
            data: Vec::new(),
            offsets,
        }
    }

    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn offsets(&self) -> &[usize] {
        &self.offsets
    }

    #[cfg(test)]
    pub(crate) fn from_rows(rows: Vec<Vec<u8>>) -> Result<Self, StringReadError> {
        checked_rows(rows.len())?;
        let total_bytes = rows
            .iter()
            .try_fold(0usize, |size, row| checked_end(size, row.len() as u64))?;
        let mut column = Self::new(rows.len());
        column.data.reserve(total_bytes);
        for row in rows {
            // The preceding checked sum validated every row and the total.
            // No need to check the same byte bound again during flattening.
            let end = column.data.len() + row.len();
            column.data.extend_from_slice(&row);
            column.offsets.push(end);
        }
        Ok(column)
    }

    /// Construct a bounded column for incremental row appends.
    pub(crate) fn builder(rows: usize) -> Result<Self, StringReadError> {
        checked_rows(rows)?;
        Ok(Self::new(rows))
    }

    /// Append a decoded row directly to the final contiguous data buffer.
    pub(crate) fn push_row(&mut self, row: &[u8]) -> Result<(), StringReadError> {
        checked_rows(self.len() + 1)?;
        let end = checked_end(self.data.len(), row.len() as u64)?;
        self.data.extend_from_slice(row);
        self.offsets.push(end);
        Ok(())
    }

    fn row_bounds(&self, row: usize) -> Option<(usize, usize)> {
        // Decoding keeps an extra terminal offset for every valid row.
        if row >= self.len() {
            return None;
        }
        Some((self.offsets[row], self.offsets[row + 1]))
    }

    pub fn get(&self, row: usize) -> Option<&[u8]> {
        let (begin, end) = self.row_bounds(row)?;
        self.data.get(begin..end)
    }

    /// Return a string's byte length without touching its payload.
    /// The decoder validates monotone offsets when building the column.
    pub fn byte_length(&self, row: usize) -> Option<usize> {
        let (begin, end) = self.row_bounds(row)?;
        Some(end - begin)
    }

    fn push<R: Read>(&mut self, reader: &mut R, length: u64) -> Result<(), StringReadError> {
        let next = checked_end(self.data.len(), length)?;
        let begin = self.data.len();
        self.data.resize(next, 0);
        reader.read_exact(&mut self.data[begin..next])?;
        self.offsets.push(next);
        Ok(())
    }
}

#[derive(Debug)]
pub enum StringReadError {
    Io(io::Error),
    TooManyRows(usize),
    TooManyBytes,
    InvalidVarUInt,
}

impl fmt::Display for StringReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "String data: {error}"),
            Self::TooManyRows(rows) => write!(f, "String batch exceeds the row limit: {rows}"),
            Self::TooManyBytes => write!(f, "String batch exceeds the 64 MiB size limit"),
            Self::InvalidVarUInt => write!(f, "invalid String length encoding"),
        }
    }
}

impl Error for StringReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for StringReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod buffered_legacy_string_tests {
    use super::*;

    struct BufferOnly<'a>(&'a [u8]);

    impl Read for BufferOnly<'_> {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("buffered string decoder must not invoke Read::read")
        }
    }

    impl BufRead for BufferOnly<'_> {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Ok(self.0)
        }

        fn consume(&mut self, amt: usize) {
            self.0 = &self.0[amt..];
        }
    }

    #[test]
    fn reads_payloads_without_per_row_reads_or_read_ahead() {
        let mut encoded = vec![0, 3, b'a', 0xff, 0, 0x80, 1];
        encoded.extend_from_slice(&[b'z'; 128]);
        encoded.push(42);
        let mut source = BufferOnly(&encoded);
        let column = read_legacy_strings_buffered(&mut source, 3).unwrap();
        assert_eq!(column.offsets(), &[0, 0, 3, 131]);
        assert_eq!(column.get(0), Some(b"".as_slice()));
        assert_eq!(column.get(1), Some([b'a', 0xff, 0].as_slice()));
        assert_eq!(column.get(2), Some([b'z'; 128].as_slice()));
        assert_eq!(source.0, &[42]);
    }

    #[test]
    fn single_byte_lengths_consume_only_their_own_prefix() {
        for length in [0u8, 1, 63, 127] {
            let bytes = [length, 0xaa];
            let mut source = BufferOnly(&bytes);
            assert_eq!(
                read_varuint_buffered(&mut source).unwrap(),
                u64::from(length)
            );
            assert_eq!(source.0, &[0xaa]);
        }
    }

    #[test]
    fn short_buffers_and_invalid_lengths_match_generic_reader() {
        for encoded in [
            vec![0x80],
            vec![0x80, 0],
            vec![3, b'a'],
            vec![0x80; 10],
            vec![0xff; 9].into_iter().chain([2]).collect(),
        ] {
            let mut generic = encoded.as_slice();
            let mut buffered = std::io::BufReader::with_capacity(1, encoded.as_slice());
            let old = read_legacy_strings(&mut generic, 1).unwrap_err();
            let new = read_legacy_strings_buffered(&mut buffered, 1).unwrap_err();
            match (old, new) {
                (StringReadError::InvalidVarUInt, StringReadError::InvalidVarUInt) => {}
                (StringReadError::Io(a), StringReadError::Io(b)) => {
                    assert_eq!(a.kind(), b.kind());
                }
                (a, b) => panic!("different legacy String errors: {a:?} vs {b:?}"),
            }
        }
    }

    #[test]
    fn buffered_decoder_matches_generic_reader_across_short_buffers() {
        let mut encoded = Vec::new();
        for row in 0..8192_usize {
            let length = row % 129;
            if length < 128 {
                encoded.push(length as u8);
            } else {
                encoded.extend_from_slice(&[0x80, 1]);
            }
            encoded.extend(std::iter::repeat_n((row % 251) as u8, length));
        }
        let mut generic = encoded.as_slice();
        let baseline = read_legacy_strings(&mut generic, 8192).unwrap();
        for capacity in [1, 7, 128, 8192] {
            let mut reader = std::io::BufReader::with_capacity(capacity, encoded.as_slice());
            let optimized = read_legacy_strings_buffered(&mut reader, 8192).unwrap();
            assert_eq!(baseline, optimized, "buffer capacity {capacity}");
        }
    }
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

    #[test]
    fn from_rows_preserves_boundaries_with_preallocated_payload() {
        let column =
            StringColumn::from_rows(vec![vec![b'a'; 1024], Vec::new(), vec![b'z'; 4096]]).unwrap();
        assert_eq!(column.len(), 3);
        assert_eq!(column.offsets(), &[0, 1024, 1024, 5120]);
        assert_eq!(column.get(2), Some(&vec![b'z'; 4096][..]));
        assert!(column.data.capacity() >= column.data.len());
    }

    fn check_values(column: &StringColumn) {
        assert_eq!(column.len(), 3);
        assert!(!column.is_empty());
        assert_eq!(column.offsets(), &[0, 0, 3, 131]);
        assert_eq!(column.get(0), Some(&b""[..]));
        assert_eq!(column.get(1), Some(&[b'a', 0xff, 0][..]));
        assert_eq!(column.get(2), Some(&[b'z'; 128][..]));
        assert_eq!(column.get(3), None);
        assert_eq!(column.get(usize::MAX), None);
    }

    #[test]
    fn string_byte_lengths_only_read_offset_boundaries() {
        let strings = read_legacy_strings(&mut &[0, 3, b'f', b'o', b'o', 1, b'z'][..], 3).unwrap();
        assert_eq!(strings.byte_length(0), Some(0));
        assert_eq!(strings.byte_length(1), Some(3));
        assert_eq!(strings.byte_length(2), Some(1));
        assert_eq!(strings.byte_length(3), None);
        assert_eq!(strings.byte_length(usize::MAX), None);
    }

    #[test]
    fn sized_string_offsets_use_one_read_for_8192_rows() {
        struct Counting<'a> {
            bytes: &'a [u8],
            reads: usize,
        }
        impl Read for Counting<'_> {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                self.reads += 1;
                self.bytes.read(out)
            }
        }
        let sizes = vec![0u8; 8192 * SIZE_ENTRY_BYTES];
        let mut source = Counting {
            bytes: &sizes,
            reads: 0,
        };
        let strings = read_sized_strings(&mut source, &mut io::empty(), 8192).unwrap();
        assert_eq!(strings.len(), 8192);
        assert_eq!(source.reads, 1);
    }

    #[test]
    fn reads_legacy_single_stream_with_raw_bytes_and_short_reads() {
        let mut content = vec![0, 3, b'a', 0xff, 0, 0x80, 1];
        content.extend_from_slice(&[b'z'; 128]);
        content.push(42);
        let mut reader = Bytewise(Cursor::new(content));
        let column = read_legacy_strings(&mut reader, 3).unwrap();
        check_values(&column);
        let mut trailing = [0];
        reader.read_exact(&mut trailing).unwrap();
        assert_eq!(trailing, [42]);
    }

    #[test]
    fn reads_position_independent_size_stream_as_little_endian_u64() {
        let mut sizes = Vec::new();
        for size in [0u64, 3, 128] {
            sizes.extend_from_slice(&size.to_le_bytes());
        }
        sizes.push(77);
        let mut bytes = vec![b'a', 0xff, 0];
        bytes.extend_from_slice(&[b'z'; 128]);
        bytes.push(42);

        let mut size_reader = Bytewise(Cursor::new(sizes));
        let mut data_reader = Bytewise(Cursor::new(bytes));
        let column = read_sized_strings(&mut size_reader, &mut data_reader, 3).unwrap();
        check_values(&column);
        let mut trailing = [0];
        size_reader.read_exact(&mut trailing).unwrap();
        assert_eq!(trailing, [77]);
        data_reader.read_exact(&mut trailing).unwrap();
        assert_eq!(trailing, [42]);
    }

    #[test]
    fn batches_size_reads_and_leaves_trailing_bytes_unconsumed() {
        struct CountingReader<R> {
            inner: R,
            reads: usize,
        }

        impl<R: Read> Read for CountingReader<R> {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.reads += 1;
                self.inner.read(bytes)
            }
        }

        let rows = 2050;
        let mut encoded = Vec::new();
        for row in 0..rows {
            let length: u64 = if row % 1024 == 0 { 1 } else { 0 };
            encoded.extend_from_slice(&length.to_le_bytes());
        }
        encoded.push(99);

        let mut sizes = CountingReader {
            inner: Cursor::new(encoded),
            reads: 0,
        };
        let mut data = &b"abc"[..];
        let column = read_sized_strings(&mut sizes, &mut data, rows).unwrap();
        assert_eq!(column.len(), rows);
        assert_eq!(column.data(), b"abc");
        assert_eq!(column.get(0), Some(&b"a"[..]));
        assert_eq!(column.get(1024), Some(&b"b"[..]));
        assert_eq!(column.get(2048), Some(&b"c"[..]));
        assert!(sizes.reads <= 3, "expected bounded bulk size reads");
        assert!(data.is_empty());

        let mut trailing = [0];
        sizes.read_exact(&mut trailing).unwrap();
        assert_eq!(trailing, [99]);
    }

    #[test]
    fn fails_on_missing_legacy_payload_or_partial_varuint() {
        for mut content in [&[3, b'a'][..], &[0x80][..]] {
            assert!(matches!(
                read_legacy_strings(&mut content, 1),
                Err(StringReadError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
            ));
        }
    }

    #[test]
    fn fails_on_short_size_or_data_stream() {
        let mut sizes = &[2, 0, 0, 0, 0, 0, 0][..];
        assert!(matches!(
            read_sized_strings(&mut sizes, &mut io::empty(), 1),
            Err(StringReadError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
        ));

        let encoded = 2u64.to_le_bytes();
        let mut sizes = encoded.as_slice();
        assert!(matches!(
            read_sized_strings(&mut sizes, &mut &[1u8][..], 1),
            Err(StringReadError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn rejects_overflow_noncanonical_lengths_and_oversized_data() {
        for length in [
            vec![0x80, 0],
            vec![0x80; 10],
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2],
        ] {
            assert!(matches!(
                read_legacy_strings(&mut length.as_slice(), 1),
                Err(StringReadError::InvalidVarUInt)
            ));
        }
        let too_large = [0x80, 0x80, 0x80, 0x80, 1];
        assert!(matches!(
            read_legacy_strings(&mut too_large.as_slice(), 1),
            Err(StringReadError::TooManyBytes)
        ));
        let encoded = u64::MAX.to_le_bytes();
        let mut sizes = encoded.as_slice();
        assert!(matches!(
            read_sized_strings(&mut sizes, &mut io::empty(), 1),
            Err(StringReadError::TooManyBytes)
        ));
    }

    #[test]
    fn validates_total_size_before_reading_any_data() {
        let mut sizes = Vec::new();
        sizes.extend_from_slice(&(MAX_BATCH_BYTES as u64).to_le_bytes());
        sizes.extend_from_slice(&1u64.to_le_bytes());
        let mut data = &[8u8][..];
        assert!(matches!(
            read_sized_strings(&mut sizes.as_slice(), &mut data, 2),
            Err(StringReadError::TooManyBytes)
        ));
        assert_eq!(data, &[8]);
    }

    #[test]
    fn rejects_untrusted_row_counts_before_allocations() {
        assert!(matches!(
            read_legacy_strings(&mut io::empty(), MAX_BATCH_ROWS + 1),
            Err(StringReadError::TooManyRows(_))
        ));
        assert!(matches!(
            read_sized_strings(&mut io::empty(), &mut io::empty(), MAX_BATCH_ROWS + 1),
            Err(StringReadError::TooManyRows(_))
        ));
    }

    #[test]
    fn reads_empty_batches_without_consuming_streams() {
        let mut sizes = &[42u8][..];
        let mut data = &[43u8][..];
        let column = read_sized_strings(&mut sizes, &mut data, 0).unwrap();
        assert!(column.is_empty());
        assert_eq!(column.offsets(), &[0]);
        assert_eq!(sizes, &[42]);
        assert_eq!(data, &[43]);

        let legacy = read_legacy_strings(&mut io::empty(), 0).unwrap();
        assert!(legacy.is_empty());
    }
}

#[cfg(test)]
mod incremental_row_builder_tests {
    use super::*;

    #[test]
    fn appending_rows_preserves_empty_entries_and_offsets() {
        let mut column = StringColumn::builder(3).unwrap();
        column.push_row(b"hello").unwrap();
        column.push_row(b"").unwrap();
        column.push_row(b"world").unwrap();
        assert_eq!(column.offsets(), &[0, 5, 5, 10]);
        assert_eq!(column.get(0), Some(&b"hello"[..]));
        assert_eq!(column.get(1), Some(&b""[..]));
        assert_eq!(column.get(2), Some(&b"world"[..]));
    }
}

#[cfg(test)]
mod buffered_varuint_bulk_tests {
    use super::*;

    struct OneBuffer<'a> {
        remaining: &'a [u8],
        fills: usize,
    }

    impl Read for OneBuffer<'_> {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("buffered varint must use BufRead")
        }
    }

    impl BufRead for OneBuffer<'_> {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            self.fills += 1;
            Ok(self.remaining)
        }

        fn consume(&mut self, count: usize) {
            self.remaining = &self.remaining[count..];
        }
    }

    #[test]
    fn multi_byte_varuint_reads_one_buffer_and_preserves_following_data() {
        let input = [0x80, 0x80, 0x80, 0x80, 0x80, 1, 0xfe];
        let mut reader = OneBuffer {
            remaining: &input,
            fills: 0,
        };
        assert_eq!(read_varuint_buffered(&mut reader).unwrap(), 1u64 << 35);
        assert_eq!(reader.fills, 1);
        assert_eq!(reader.remaining, &[0xfe]);
    }

    #[test]
    fn corrupt_tail_is_consumed_and_rejected() {
        let input = [0x80, 0];
        let mut reader = OneBuffer {
            remaining: &input,
            fills: 0,
        };
        assert!(matches!(
            read_varuint_buffered(&mut reader),
            Err(StringReadError::InvalidVarUInt)
        ));
        assert!(reader.remaining.is_empty());
    }
}

#[cfg(test)]
mod preflight_flattening_tests {
    use super::*;

    #[test]
    fn fast_row_access_preserves_empty_long_and_out_of_range_values() {
        let input = vec![
            vec![],
            b"abc".to_vec(),
            vec![0, b'\\', 0xff, 0x7f],
            vec![b'x'; 8192],
            vec![],
        ];
        let values = StringColumn::from_rows(input.clone()).unwrap();
        for (row, expected) in input.iter().enumerate() {
            assert_eq!(values.get(row), Some(expected.as_slice()));
            assert_eq!(values.byte_length(row), Some(expected.len()));
        }
        for row in [input.len(), input.len() + 1, usize::MAX] {
            assert_eq!(values.get(row), None);
            assert_eq!(values.byte_length(row), None);
        }
    }

    #[test]
    fn prevalidated_rows_keep_exact_offsets_and_payloads() {
        let rows = (0..8_192).map(|i| vec![b'x'; i % 11]).collect::<Vec<_>>();
        let expected_bytes = rows.iter().map(Vec::len).sum::<usize>();
        let column = StringColumn::from_rows(rows).unwrap();
        assert_eq!(column.len(), 8_192);
        assert_eq!(column.data.len(), expected_bytes);
        assert_eq!(column.offsets.last(), Some(&expected_bytes));
        for row in 0..8_192 {
            assert_eq!(column.get(row).unwrap(), vec![b'x'; row % 11]);
        }
    }
}

#[cfg(test)]
mod sized_string_mark_buffer_tests {
    use super::*;
    use std::io::{Cursor, Read};

    #[test]
    fn decodes_across_all_buffer_thresholds_without_read_ahead() {
        for rows in [0usize, 1, 64, 65, 1024, 1025, 8192, 8193] {
            let mut sizes = vec![];
            for index in 0..rows {
                sizes.extend_from_slice(&((index % 3) as u64).to_le_bytes());
            }
            let mut payload = (0..rows)
                .flat_map(|index| std::iter::repeat_n((index % 251) as u8, index % 3))
                .collect::<Vec<_>>();
            let bytes = payload.len();
            sizes.extend_from_slice(&u64::MAX.to_le_bytes());
            payload.push(0xfe);
            let mut sizes = Cursor::new(sizes);
            let mut data = Cursor::new(payload);
            let result = read_sized_strings(&mut sizes, &mut data, rows).unwrap();
            assert_eq!(result.len(), rows);
            assert_eq!(result.data().len(), bytes);
            assert_eq!(sizes.position(), (rows * 8) as u64);
            assert_eq!(data.position(), bytes as u64);
            for index in 0..rows {
                let expected =
                    std::iter::repeat_n((index % 251) as u8, index % 3).collect::<Vec<_>>();
                assert_eq!(result.get(index), Some(expected.as_slice()));
            }
            let mut tail = [0u8; 8];
            sizes.read_exact(&mut tail).unwrap();
            assert_eq!(tail, u64::MAX.to_le_bytes());
            let mut data_tail = [0u8; 1];
            data.read_exact(&mut data_tail).unwrap();
            assert_eq!(data_tail, [0xfe]);
        }
    }

    #[test]
    fn rejects_truncated_size_streams_at_each_threshold() {
        for rows in [1, 65, 1025, 8193] {
            let size_bytes = vec![0u8; rows * 8 - 1];
            let mut sizes = size_bytes.as_slice();
            let mut data = std::io::empty();
            assert!(matches!(
                read_sized_strings(&mut sizes, &mut data, rows),
                Err(StringReadError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
            ));
        }
    }
}

#[cfg(test)]
mod one_buffer_legacy_string_tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    #[test]
    fn short_rows_and_split_payloads_decode_identically() {
        let mut encoded = Vec::new();
        let expected = (0..4096)
            .map(|index| vec![(index % 251) as u8; index % 31])
            .collect::<Vec<_>>();
        for row in &expected {
            encoded.push(row.len() as u8);
            encoded.extend_from_slice(row);
        }
        // A narrow buffer forces the fallback across length/payload boundaries.
        for capacity in [1usize, 2, 7, 64, 4096] {
            let mut source = BufReader::with_capacity(capacity, Cursor::new(&encoded));
            let decoded = read_legacy_strings_buffered(&mut source, expected.len()).unwrap();
            for (row, expected) in expected.iter().enumerate() {
                assert_eq!(decoded.get(row), Some(expected.as_slice()));
            }
        }
    }

    #[test]
    fn truncated_short_payload_still_fails() {
        let mut source = BufReader::with_capacity(32, Cursor::new([4u8, b'a', b'b']));
        assert!(matches!(
            read_legacy_strings_buffered(&mut source, 1),
            Err(StringReadError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
        ));
    }
}
