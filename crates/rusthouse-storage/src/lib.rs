mod arrays;
mod columns;
mod compression;
mod granules;
mod mark_validation;
mod marks;
mod nullable;
mod numeric;
mod primary_index;
mod stream;
mod strings;

pub use arrays::{
    ArrayOffsets, ArrayOffsetsEncoding, ArrayReadError, NumericArrayColumn, read_array_offsets,
    read_numeric_array_column,
};
pub use columns::parse_columns;
pub use compression::{CompressedBlock, CompressedBlockReader, DecodedBlock};
pub use granules::{read_mark_bytes, read_u64_granule};
pub use mark_validation::validate_wide_marks;
pub use marks::{WideMark, WideMarkFormat, WideMarks, read_wide_marks};
pub use nullable::{
    NullableColumn, NullableReadError, read_null_map, read_nullable_legacy_strings,
    read_nullable_numeric_column, read_nullable_sized_strings,
};
pub use numeric::{NumericColumn, NumericReadError, read_numeric_column};
pub use primary_index::{FixedPrimaryIndex, read_fixed_primary_index};
pub use stream::UInt64ColumnReader;
pub use strings::{StringColumn, StringReadError, read_legacy_strings, read_sized_strings};

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartMetadata {
    pub path: PathBuf,
    pub rows: u64,
    pub columns: Vec<Column>,
}

#[derive(Debug)]
pub enum PartError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    InvalidCount(String),
    InvalidColumns {
        line: usize,
        message: &'static str,
    },
    InvalidMarksLength {
        path: PathBuf,
        size: usize,
        record_size: usize,
    },
    EmptyMarksFile(PathBuf),
    InvalidMarkOffset {
        path: PathBuf,
        index: usize,
        reason: &'static str,
    },
    InvalidCompressedBlock {
        path: PathBuf,
        offset: u64,
        reason: &'static str,
    },
    InvalidPrimaryIndex {
        path: PathBuf,
        reason: &'static str,
    },
    InvalidColumnLength {
        path: PathBuf,
        width: usize,
        trailing_bytes: usize,
    },
    InvalidWideMark {
        compressed_offset: u64,
        decompressed_offset: u64,
        reason: &'static str,
    },
    InvalidGranuleIndex {
        index: usize,
        marks: usize,
    },
    UnsupportedMarksFormat(PathBuf),
}

impl fmt::Display for PartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::InvalidCount(value) => write!(f, "invalid count.txt row count: {value:?}"),
            Self::InvalidColumns { line, message } => {
                write!(f, "invalid columns.txt at line {line}: {message}")
            }
            Self::InvalidMarksLength {
                path,
                size,
                record_size,
            } => write!(
                f,
                "{}: {size} bytes is not a multiple of {record_size} bytes per mark",
                path.display()
            ),
            Self::EmptyMarksFile(path) => write!(f, "{}: empty marks file", path.display()),
            Self::InvalidMarkOffset {
                path,
                index,
                reason,
            } => write!(f, "{}: invalid mark {index}: {reason}", path.display()),
            Self::InvalidCompressedBlock {
                path,
                offset,
                reason,
            } => {
                write!(
                    f,
                    "{}: invalid compressed block at byte {offset}: {reason}",
                    path.display()
                )
            }
            Self::InvalidPrimaryIndex { path, reason } => {
                write!(f, "{}: invalid primary index: {reason}", path.display())
            }
            Self::InvalidColumnLength {
                path,
                width,
                trailing_bytes,
            } => write!(
                f,
                "{}: incomplete {width}-byte column value ({trailing_bytes} trailing bytes)",
                path.display()
            ),
            Self::InvalidWideMark {
                compressed_offset,
                decompressed_offset,
                reason,
            } => write!(
                f,
                "invalid wide mark (compressed={compressed_offset}, decompressed={decompressed_offset}): {reason}"
            ),
            Self::InvalidGranuleIndex { index, marks } => {
                write!(f, "granule index {index} is outside {marks} marks")
            }
            Self::UnsupportedMarksFormat(path) => write!(
                f,
                "{}: only wide-part .mrk, .mrk2, .cmrk, and .cmrk2 files are supported",
                path.display()
            ),
        }
    }
}

impl Error for PartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn inspect_part(path: impl AsRef<Path>) -> Result<PartMetadata, PartError> {
    let path = path.as_ref();
    let rows = parse_count(&read_metadata(&path.join("count.txt"))?)?;
    let columns = parse_columns(&read_metadata(&path.join("columns.txt"))?)?;

    Ok(PartMetadata {
        path: path.to_path_buf(),
        rows,
        columns,
    })
}

fn read_metadata(path: &Path) -> Result<String, PartError> {
    fs::read_to_string(path).map_err(|source| PartError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn parse_count(content: &str) -> Result<u64, PartError> {
    content
        .strip_suffix('\n')
        .unwrap_or(content)
        .parse()
        .map_err(|_| PartError::InvalidCount(content.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspects_fixture_part_without_writing() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/all_1_1_0");
        let metadata = inspect_part(&path).unwrap();
        assert_eq!(metadata.path, path);
        assert_eq!(metadata.rows, 42);
        assert_eq!(metadata.columns.len(), 2);
        assert_eq!(metadata.columns[0].name, "id");
        assert_eq!(metadata.columns[1].data_type, "LowCardinality(String)");
    }

    #[test]
    fn reports_missing_count_file() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/missing");
        assert!(matches!(inspect_part(path), Err(PartError::Io { .. })));
    }

    #[test]
    fn accepts_plain_and_newline_terminated_counts() {
        assert_eq!(parse_count("42").unwrap(), 42);
        assert_eq!(parse_count("42\n").unwrap(), 42);
        assert_eq!(parse_count("18446744073709551615\n").unwrap(), u64::MAX);
    }

    #[test]
    fn rejects_malformed_counts() {
        for input in [
            "",
            "\n",
            " 42\n",
            "42 \n",
            "42\n\n",
            "-1\n",
            "18446744073709551616\n",
        ] {
            assert!(
                matches!(parse_count(input), Err(PartError::InvalidCount(_))),
                "{input:?}"
            );
        }
    }
}
