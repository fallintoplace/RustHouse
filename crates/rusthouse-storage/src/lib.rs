mod columns;

pub use columns::parse_columns;

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
    Io { path: PathBuf, source: io::Error },
    InvalidCount(String),
    InvalidColumns { line: usize, message: &'static str },
}

impl fmt::Display for PartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::InvalidCount(value) => write!(f, "invalid count.txt row count: {value:?}"),
            Self::InvalidColumns { line, message } => {
                write!(f, "invalid columns.txt at line {line}: {message}")
            }
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
        for input in ["", "\n", " 42\n", "42 \n", "42\n\n", "-1\n", "18446744073709551616\n"] {
            assert!(matches!(parse_count(input), Err(PartError::InvalidCount(_))), "{input:?}");
        }
    }
}
