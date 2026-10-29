use crate::fixed_width::MAX_BATCH_BYTES;
use crate::{CompressedBlockReader, PartError, WideMark, read_mark_bytes};
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Seek};

const MAX_BATCH_ROWS: usize = 1_000_000;
const MAX_FIXED_STRING_WIDTH: usize = 0x00ff_ffff;

#[derive(Debug, PartialEq, Eq)]
pub struct FixedStringColumn {
    data: Vec<u8>,
    width: usize,
}

impl FixedStringColumn {
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn len(&self) -> usize {
        self.data.len() / self.width
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn get(&self, row: usize) -> Option<&[u8]> {
        let start = row.checked_mul(self.width)?;
        self.data.get(start..start.checked_add(self.width)?)
    }
}

#[derive(Debug)]
pub enum FixedStringReadError {
    InvalidType(String),
    TooManyRows(usize),
    TooManyBytes,
    Io(io::Error),
}

impl fmt::Display for FixedStringReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidType(data_type) => write!(f, "invalid FixedString type: {data_type}"),
            Self::TooManyRows(rows) => write!(f, "FixedString batch exceeds the row limit: {rows}"),
            Self::TooManyBytes => write!(f, "FixedString batch exceeds the 64 MiB size limit"),
            Self::Io(source) => write!(f, "unable to read FixedString column: {source}"),
        }
    }
}

impl Error for FixedStringReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(source) => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for FixedStringReadError {
    fn from(source: io::Error) -> Self {
        Self::Io(source)
    }
}

fn parse_fixed_width(data_type: &str) -> Result<usize, FixedStringReadError> {
    let width = data_type
        .strip_prefix("FixedString(")
        .and_then(|value| value.strip_suffix(')'))
        .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|width| (1..=MAX_FIXED_STRING_WIDTH).contains(width))
        .ok_or_else(|| FixedStringReadError::InvalidType(data_type.to_owned()))?;
    Ok(width)
}

pub(crate) fn checked_size(
    data_type: &str,
    rows: usize,
) -> Result<(usize, usize), FixedStringReadError> {
    let width = parse_fixed_width(data_type)?;
    if rows > MAX_BATCH_ROWS {
        return Err(FixedStringReadError::TooManyRows(rows));
    }
    let bytes = rows
        .checked_mul(width)
        .filter(|bytes| *bytes <= MAX_BATCH_BYTES)
        .ok_or(FixedStringReadError::TooManyBytes)?;
    Ok((width, bytes))
}

pub fn read_fixed_strings<R: Read>(
    reader: &mut R,
    data_type: &str,
    rows: usize,
) -> Result<FixedStringColumn, FixedStringReadError> {
    let (width, bytes) = checked_size(data_type, rows)?;
    let mut data = vec![0; bytes];
    reader.read_exact(&mut data)?;
    Ok(FixedStringColumn { data, width })
}

pub fn read_fixed_string_granule<R: Read + Seek>(
    reader: &mut CompressedBlockReader<R>,
    mark: &WideMark,
    data_type: &str,
) -> Result<FixedStringColumn, PartError> {
    let rows = mark.rows.ok_or(PartError::InvalidWideMark {
        compressed_offset: mark.compressed_offset,
        decompressed_offset: mark.decompressed_offset,
        reason: "FixedString granules require adaptive marks",
    })?;
    let rows = usize::try_from(rows).map_err(|_| PartError::InvalidWideMark {
        compressed_offset: mark.compressed_offset,
        decompressed_offset: mark.decompressed_offset,
        reason: "granule size overflows",
    })?;
    let (width, bytes) = checked_size(data_type, rows).map_err(PartError::FixedStringRead)?;
    let data = read_mark_bytes(reader, mark, bytes)?;
    Ok(FixedStringColumn { data, width })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct OneByte<R>(R);

    impl<R: Read> Read for OneByte<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let count = buf.len().min(1);
            self.0.read(&mut buf[..count])
        }
    }

    fn checked_block(method: u8, data: &[u8]) -> Vec<u8> {
        let payload = if method == 0x82 {
            lz4_flex::block::compress(data)
        } else {
            data.to_vec()
        };
        let mut encoded = vec![method];
        encoded.extend_from_slice(&(9u32 + payload.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&(data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&payload);
        let mut result = u128::from(ch_cityhash102::cityhash128(&encoded))
            .to_le_bytes()
            .to_vec();
        result.extend_from_slice(&encoded);
        result
    }

    fn mark(compressed_offset: u64, decompressed_offset: u64, rows: Option<u64>) -> WideMark {
        WideMark {
            compressed_offset,
            decompressed_offset,
            rows,
        }
    }

    fn reader(data: Vec<u8>) -> CompressedBlockReader<Cursor<Vec<u8>>> {
        CompressedBlockReader::new(Cursor::new(data), "strings.bin").unwrap()
    }

    #[test]
    fn reads_fixed_width_binary_values_without_trimming_padding() {
        let mut input = OneByte(Cursor::new([b'a', 0, 0, 0xff, b'b', 0, 42]));
        let values = read_fixed_strings(&mut input, "FixedString(3)", 2).unwrap();
        assert_eq!(values.width(), 3);
        assert_eq!(values.len(), 2);
        assert!(!values.is_empty());
        assert_eq!(values.data(), &[b'a', 0, 0, 0xff, b'b', 0]);
        assert_eq!(values.get(0), Some(&[b'a', 0, 0][..]));
        assert_eq!(values.get(1), Some(&[0xff, b'b', 0][..]));
        assert_eq!(values.get(2), None);
        assert_eq!(values.get(usize::MAX), None);
        let mut trailing = [0];
        input.read_exact(&mut trailing).unwrap();
        assert_eq!(trailing, [42]);
    }

    #[test]
    fn reads_marked_strings_across_checked_lz4_and_plain_blocks() {
        let mut blocks = checked_block(0x82, b"abcde");
        let second_offset = blocks.len() as u64;
        blocks.extend(checked_block(0x02, b"fghijkl"));
        let mut input = reader(blocks);

        let values =
            read_fixed_string_granule(&mut input, &mark(0, 2, Some(3)), "FixedString(3)").unwrap();
        assert_eq!(values.get(0), Some(&b"cde"[..]));
        assert_eq!(values.get(1), Some(&b"fgh"[..]));
        assert_eq!(values.get(2), Some(&b"ijk"[..]));

        let values = read_fixed_string_granule(
            &mut input,
            &mark(second_offset, 1, Some(2)),
            "FixedString(3)",
        )
        .unwrap();
        assert_eq!(values.get(0), Some(&b"ghi"[..]));
        assert_eq!(values.get(1), Some(&b"jkl"[..]));
    }

    #[test]
    fn rejects_invalid_widths_and_batches_before_reading() {
        struct NoRead;

        impl Read for NoRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("reader should not be accessed");
            }
        }

        for data_type in [
            "String",
            "FixedString(0)",
            "FixedString(-1)",
            "FixedString(+3)",
            "FixedString(3 )",
            "FixedString( 3)",
            "FixedString()",
            "FixedString(16777216)",
            "FixedString(18446744073709551616)",
        ] {
            assert!(
                matches!(
                    read_fixed_strings(&mut NoRead, data_type, 1),
                    Err(FixedStringReadError::InvalidType(_))
                ),
                "{data_type}"
            );
        }
        assert!(matches!(
            read_fixed_strings(&mut NoRead, "FixedString(1)", MAX_BATCH_ROWS + 1),
            Err(FixedStringReadError::TooManyRows(_))
        ));
        assert!(matches!(
            read_fixed_strings(&mut NoRead, "FixedString(1000000)", 100),
            Err(FixedStringReadError::TooManyBytes)
        ));
    }

    #[test]
    fn rejects_truncated_data_and_corrupted_blocks() {
        assert!(matches!(
            read_fixed_strings(&mut &[1, 2][..], "FixedString(3)", 1),
            Err(FixedStringReadError::Io(error))
                if error.kind() == io::ErrorKind::UnexpectedEof
        ));

        let mut blocks = checked_block(0x82, b"abc");
        let mut corrupt = checked_block(0x02, b"def");
        corrupt[25] ^= 1;
        blocks.extend(corrupt);
        let mut input = reader(blocks);
        assert!(matches!(
            read_fixed_string_granule(&mut input, &mark(0, 0, Some(2)), "FixedString(3)"),
            Err(PartError::InvalidCompressedBlock {
                reason: "checksum mismatch",
                ..
            })
        ));
    }

    #[test]
    fn rejects_unbounded_marks_and_allows_empty_batches() {
        let mut input = reader(Vec::new());
        assert!(matches!(
            read_fixed_string_granule(&mut input, &mark(0, 0, None), "FixedString(2)"),
            Err(PartError::InvalidWideMark {
                reason: "FixedString granules require adaptive marks",
                ..
            })
        ));
        assert!(matches!(
            read_fixed_string_granule(&mut input, &mark(0, 0, Some(u64::MAX)), "FixedString(2)"),
            Err(PartError::FixedStringRead(
                FixedStringReadError::TooManyRows(_)
            ))
        ));
        let empty =
            read_fixed_string_granule(&mut input, &mark(u64::MAX, 0, Some(0)), "FixedString(4)")
                .unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.width(), 4);
        assert_eq!(empty.len(), 0);
        assert!(empty.get(0).is_none());

        let mut unread = &[42u8][..];
        let empty = read_fixed_strings(&mut unread, "FixedString(4)", 0).unwrap();
        assert!(empty.is_empty());
        assert_eq!(unread, &[42]);
    }
    #[test]
    fn preserves_fixed_string_row_boundaries_across_short_reads() {
        let rows = 16 * 1024 / 3 + 1;
        let mut bytes = vec![0u8; rows * 3];
        bytes[..3].copy_from_slice(&[1, 2, 3]);
        bytes[(rows - 1) * 3..].copy_from_slice(&[4, 5, 6]);
        let mut reader = OneByte(Cursor::new(bytes));
        let column = read_fixed_strings(&mut reader, "FixedString(3)", rows).unwrap();

        assert_eq!(column.len(), rows);
        assert_eq!(column.get(0), Some(&[1, 2, 3][..]));
        assert_eq!(column.get(1), Some(&[0, 0, 0][..]));
        assert_eq!(column.get(rows - 1), Some(&[4, 5, 6][..]));
    }

    #[test]
    fn reads_consecutive_fixed_string_batches() {
        let mut bytes = &b"abcdefg"[..];
        let first = read_fixed_strings(&mut bytes, "FixedString(2)", 1).unwrap();
        let second = read_fixed_strings(&mut bytes, "FixedString(1)", 3).unwrap();

        assert_eq!(first.get(0), Some(&b"ab"[..]));
        assert_eq!(second.get(0), Some(&b"c"[..]));
        assert_eq!(second.get(2), Some(&b"e"[..]));
        assert_eq!(bytes, b"fg");
    }
}
