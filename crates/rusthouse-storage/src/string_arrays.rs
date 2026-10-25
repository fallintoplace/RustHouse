use crate::{
    ArrayOffsets, ArrayOffsetsEncoding, ArrayReadError, StringColumn, StringReadError,
    read_array_offsets, read_legacy_strings, read_sized_strings,
};
use std::error::Error;
use std::fmt;
use std::io::Read;
use std::ops::Range;

#[derive(Debug, PartialEq, Eq)]
pub struct StringArrayColumn {
    offsets: ArrayOffsets,
    values: StringColumn,
}

impl StringArrayColumn {
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn offsets(&self) -> &[usize] {
        self.offsets.offsets()
    }

    pub fn values(&self) -> &StringColumn {
        &self.values
    }

    pub fn row_range(&self, row: usize) -> Option<Range<usize>> {
        self.offsets.row_range(row)
    }

    pub fn last_absolute_offset(&self) -> u64 {
        self.offsets.last_absolute_offset()
    }
}

#[derive(Debug)]
pub enum StringArrayReadError {
    InvalidType(String),
    Offsets(ArrayReadError),
    Values(StringReadError),
}

impl fmt::Display for StringArrayReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidType(data_type) => {
                write!(f, "expected Array(String), got {data_type}")
            }
            Self::Offsets(source) => write!(f, "Array offsets: {source}"),
            Self::Values(source) => write!(f, "Array String values: {source}"),
        }
    }
}

impl Error for StringArrayReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidType(_) => None,
            Self::Offsets(source) => Some(source),
            Self::Values(source) => Some(source),
        }
    }
}

fn read_offsets<O: Read>(
    offsets: &mut O,
    data_type: &str,
    rows: usize,
    encoding: ArrayOffsetsEncoding,
) -> Result<ArrayOffsets, StringArrayReadError> {
    if data_type != "Array(String)" {
        return Err(StringArrayReadError::InvalidType(data_type.to_owned()));
    }
    read_array_offsets(offsets, rows, encoding).map_err(StringArrayReadError::Offsets)
}

pub fn read_legacy_string_array_column<O: Read, D: Read>(
    offsets: &mut O,
    data: &mut D,
    data_type: &str,
    rows: usize,
    encoding: ArrayOffsetsEncoding,
) -> Result<StringArrayColumn, StringArrayReadError> {
    let offsets = read_offsets(offsets, data_type, rows, encoding)?;
    let values =
        read_legacy_strings(data, offsets.element_count()).map_err(StringArrayReadError::Values)?;
    Ok(StringArrayColumn { offsets, values })
}

pub fn read_sized_string_array_column<O: Read, S: Read, D: Read>(
    offsets: &mut O,
    sizes: &mut S,
    data: &mut D,
    data_type: &str,
    rows: usize,
    encoding: ArrayOffsetsEncoding,
) -> Result<StringArrayColumn, StringArrayReadError> {
    let offsets = read_offsets(offsets, data_type, rows, encoding)?;
    let values = read_sized_strings(sizes, data, offsets.element_count())
        .map_err(StringArrayReadError::Values)?;
    Ok(StringArrayColumn { offsets, values })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    struct Bytewise<R>(R);

    impl<R: Read> Read for Bytewise<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.0.read(&mut buffer[..buffer.len().min(1)])
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
    fn reads_legacy_array_strings_with_empty_rows_and_binary_bytes() {
        let mut offsets = Bytewise(Cursor::new(encoded_offsets(&[2, 0, 1])));
        let mut data = Bytewise(Cursor::new([1, b'a', 0, 2, 0, 0xff, 77]));
        let column = read_legacy_string_array_column(
            &mut offsets,
            &mut data,
            "Array(String)",
            3,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();

        assert_eq!(column.len(), 3);
        assert!(!column.is_empty());
        assert_eq!(column.offsets(), &[0, 2, 2, 3]);
        assert_eq!(column.last_absolute_offset(), 3);
        assert_eq!(column.row_range(0), Some(0..2));
        assert_eq!(column.row_range(1), Some(2..2));
        assert_eq!(column.row_range(2), Some(2..3));
        assert_eq!(column.row_range(3), None);
        assert_eq!(column.values().get(0), Some(&b"a"[..]));
        assert_eq!(column.values().get(1), Some(&b""[..]));
        assert_eq!(column.values().get(2), Some(&[0, 0xff][..]));
        assert_eq!(offsets.0.position(), 24);
        assert_eq!(data.0.position(), 6);
    }

    #[test]
    fn reads_sized_strings_with_running_absolute_offsets() {
        let mut offsets = Bytewise(Cursor::new(encoded_offsets(&[11, 13, 20])));
        let mut sizes = Bytewise(Cursor::new(encoded_offsets(&[2, 0, 1, 999])));
        let mut data = Bytewise(Cursor::new([b'h', b'i', 0xff, 88]));
        let column = read_sized_string_array_column(
            &mut offsets,
            &mut sizes,
            &mut data,
            "Array(String)",
            2,
            ArrayOffsetsEncoding::Absolute {
                previous_offset: 10,
            },
        )
        .unwrap();

        assert_eq!(column.offsets(), &[0, 1, 3]);
        assert_eq!(column.last_absolute_offset(), 13);
        assert_eq!(column.values().get(0), Some(&b"hi"[..]));
        assert_eq!(column.values().get(1), Some(&b""[..]));
        assert_eq!(column.values().get(2), Some(&[0xff][..]));
        assert_eq!(offsets.0.position(), 16);
        assert_eq!(sizes.0.position(), 24);
        assert_eq!(data.0.position(), 3);
    }

    #[test]
    fn rejects_wrong_types_before_reading_streams() {
        for data_type in ["Array(UInt8)", "String", "Array()", "Array(String) "] {
            assert!(matches!(
                read_legacy_string_array_column(
                    &mut NoRead,
                    &mut NoRead,
                    data_type,
                    1,
                    ArrayOffsetsEncoding::Sizes,
                ),
                Err(StringArrayReadError::InvalidType(_))
            ));
        }
    }

    #[test]
    fn reports_bad_offsets_and_truncated_legacy_values() {
        let error = read_legacy_string_array_column(
            &mut &[0u8; 7][..],
            &mut NoRead,
            "Array(String)",
            1,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap_err();
        assert!(matches!(error, StringArrayReadError::Offsets(_)));

        let error = read_legacy_string_array_column(
            &mut encoded_offsets(&[2]).as_slice(),
            &mut &[1, b'x'][..],
            "Array(String)",
            1,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            StringArrayReadError::Values(StringReadError::Io(source))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn reports_truncated_size_or_data_streams() {
        let error = read_sized_string_array_column(
            &mut encoded_offsets(&[1]).as_slice(),
            &mut &[0u8; 7][..],
            &mut NoRead,
            "Array(String)",
            1,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            StringArrayReadError::Values(StringReadError::Io(source))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));

        let error = read_sized_string_array_column(
            &mut encoded_offsets(&[1]).as_slice(),
            &mut 2u64.to_le_bytes().as_slice(),
            &mut &[b'x'][..],
            "Array(String)",
            1,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            StringArrayReadError::Values(StringReadError::Io(source))
                if source.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn enforces_element_limits_before_reading_values() {
        assert!(matches!(
            read_legacy_string_array_column(
                &mut encoded_offsets(&[1_000_001]).as_slice(),
                &mut NoRead,
                "Array(String)",
                1,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(StringArrayReadError::Values(StringReadError::TooManyRows(1_000_001)))
        ));
        assert!(matches!(
            read_legacy_string_array_column(
                &mut encoded_offsets(&[64 * 1024 * 1024 + 1]).as_slice(),
                &mut NoRead,
                "Array(String)",
                1,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(StringArrayReadError::Offsets(ArrayReadError::TooManyElements(_)))
        ));
        assert!(matches!(
            read_legacy_string_array_column(
                &mut NoRead,
                &mut NoRead,
                "Array(String)",
                1_000_001,
                ArrayOffsetsEncoding::Sizes,
            ),
            Err(StringArrayReadError::Offsets(ArrayReadError::TooManyRows(_)))
        ));
    }

    #[test]
    fn empty_batches_do_not_consume_streams() {
        let column = read_sized_string_array_column(
            &mut NoRead,
            &mut NoRead,
            &mut NoRead,
            "Array(String)",
            0,
            ArrayOffsetsEncoding::Sizes,
        )
        .unwrap();
        assert!(column.is_empty());
        assert_eq!(column.offsets(), &[0]);
        assert!(column.values().is_empty());
    }
}
