use crate::fixed_width::MAX_BATCH_BYTES;
use std::error::Error;
use std::fmt;
use std::io::{self, Read};

const MAX_BATCH_ROWS: usize = 1_000_000;
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

    pub(crate) fn from_rows(rows: Vec<Vec<u8>>) -> Result<Self, StringReadError> {
        checked_rows(rows.len())?;
        let total_bytes = rows
            .iter()
            .try_fold(0usize, |size, row| checked_end(size, row.len() as u64))?;
        let mut column = Self::new(rows.len());
        column.data.reserve(total_bytes);
        for row in rows {
            let end = checked_end(column.data.len(), row.len() as u64)?;
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

    pub fn get(&self, row: usize) -> Option<&[u8]> {
        let begin = *self.offsets.get(row)?;
        let end = *self.offsets.get(row.checked_add(1)?)?;
        self.data.get(begin..end)
    }

    /// Return a string's byte length without touching its payload.
    /// The decoder validates monotone offsets when building the column.
    pub fn byte_length(&self, row: usize) -> Option<usize> {
        let begin = *self.offsets.get(row)?;
        let end = *self.offsets.get(row.checked_add(1)?)?;
        end.checked_sub(begin)
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

fn checked_rows(rows: usize) -> Result<(), StringReadError> {
    if rows > MAX_BATCH_ROWS {
        Err(StringReadError::TooManyRows(rows))
    } else {
        Ok(())
    }
}

fn checked_end(current: usize, length: u64) -> Result<usize, StringReadError> {
    let available = MAX_BATCH_BYTES - current;
    if length > available as u64 {
        return Err(StringReadError::TooManyBytes);
    }
    Ok(current + length as usize)
}

pub fn read_legacy_strings<R: Read>(
    reader: &mut R,
    rows: usize,
) -> Result<StringColumn, StringReadError> {
    checked_rows(rows)?;
    let mut values = StringColumn::new(rows);
    for _ in 0..rows {
        let length = read_varuint(reader)?;
        values.push(reader, length)?;
    }
    Ok(values)
}

fn append_sized_offsets(values: &mut StringColumn, bytes: &[u8]) -> Result<(), StringReadError> {
    let mut end = *values.offsets.last().unwrap();
    for encoded in bytes.chunks_exact(SIZE_ENTRY_BYTES) {
        let length = u64::from_le_bytes(encoded.try_into().unwrap());
        end = checked_end(end, length)?;
        values.offsets.push(end);
    }
    Ok(())
}

pub fn read_sized_strings<S: Read, D: Read>(
    sizes: &mut S,
    data: &mut D,
    rows: usize,
) -> Result<StringColumn, StringReadError> {
    checked_rows(rows)?;
    let mut values = StringColumn::new(rows);
    // Read size entries in bounded chunks without consuming bytes from the next batch.
    let mut buffer = [0u8; SIZE_ENTRY_BYTES * 8192];
    let mut decoded_rows = 0;
    while decoded_rows < rows {
        let batch_rows = (rows - decoded_rows).min(buffer.len() / SIZE_ENTRY_BYTES);
        sizes.read_exact(&mut buffer[..batch_rows * SIZE_ENTRY_BYTES])?;
        append_sized_offsets(&mut values, &buffer[..batch_rows * SIZE_ENTRY_BYTES])?;
        decoded_rows += batch_rows;
    }
    let bytes = *values.offsets.last().unwrap();
    values.data.resize(bytes, 0);
    data.read_exact(&mut values.data)?;
    Ok(values)
}

fn read_varuint<R: Read>(reader: &mut R) -> Result<u64, StringReadError> {
    let mut result = 0u64;
    for group in 0..10 {
        let mut byte = [0; 1];
        reader.read_exact(&mut byte)?;
        if group == 9 && byte[0] > 1 {
            return Err(StringReadError::InvalidVarUInt);
        }
        let value = u64::from(byte[0] & 0x7f);
        result |= value << (group * 7);
        if byte[0] & 0x80 == 0 {
            if group != 0 && value == 0 {
                return Err(StringReadError::InvalidVarUInt);
            }
            return Ok(result);
        }
    }
    Err(StringReadError::InvalidVarUInt)
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
