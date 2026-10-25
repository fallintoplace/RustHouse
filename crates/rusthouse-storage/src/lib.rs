//! Read-only access to metadata from ClickHouse MergeTree data parts.
//!
//! This crate does not decode compressed column files, verify checksums, or
//! provide a SQL engine. A part must contain modern `count.txt` and
//! `columns.txt` files to be inspected.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// ClickHouse type expression, retained as text (not yet type-checked).
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

/// Inspect a local MergeTree data part without modifying it.
///
/// Supports parts with `count.txt` and the version-1 `columns.txt` format.
/// Older part layouts without `count.txt` are currently unsupported.
pub fn inspect_part(path: impl AsRef<Path>) -> Result<PartMetadata, PartError> {
    let path = path.as_ref();
    let count_path = path.join("count.txt");
    let count = fs::read_to_string(&count_path).map_err(|source| PartError::Io {
        path: count_path,
        source,
    })?;
    let rows = count
        .trim()
        .parse::<u64>()
        .map_err(|_| PartError::InvalidCount(count.trim().to_owned()))?;

    let columns_path = path.join("columns.txt");
    let content = fs::read_to_string(&columns_path).map_err(|source| PartError::Io {
        path: columns_path,
        source,
    })?;
    let columns = parse_columns(&content)?;

    Ok(PartMetadata {
        path: path.to_path_buf(),
        rows,
        columns,
    })
}

/// Parse the version-1 `columns.txt` text representation.
///
/// Column types are preserved verbatim. SQL-style doubled backticks and common
/// ClickHouse backslash escapes in column identifiers are supported.
pub fn parse_columns(content: &str) -> Result<Vec<Column>, PartError> {
    let mut lines = content.lines();
    if lines.next() != Some("columns format version: 1") {
        return Err(invalid(1, "expected columns format version: 1"));
    }

    let header = lines
        .next()
        .ok_or_else(|| invalid(2, "missing column-count header"))?;
    let expected = header
        .strip_suffix(" columns:")
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| invalid(2, "expected '<count> columns:'"))?;

    let mut columns = Vec::new();
    for index in 0..expected {
        let line_number = index + 3;
        let line = lines
            .next()
            .ok_or_else(|| invalid(line_number, "fewer columns than declared"))?;
        columns.push(parse_column(line, line_number)?);
    }
    if lines.next().is_some() {
        return Err(invalid(expected + 3, "more columns than declared"));
    }

    Ok(columns)
}

fn invalid(line: usize, message: &'static str) -> PartError {
    PartError::InvalidColumns { line, message }
}

fn parse_column(line: &str, line_number: usize) -> Result<Column, PartError> {
    let mut chars = line
        .strip_prefix('`')
        .ok_or_else(|| invalid(line_number, "expected a backtick-quoted column name"))?
        .chars();
    let mut name = String::new();

    while let Some(ch) = chars.next() {
        match ch {
            '`' if chars.as_str().starts_with('`') => {
                chars.next();
                name.push('`');
            }
            '`' => {
                let data_type = chars
                    .as_str()
                    .strip_prefix(' ')
                    .ok_or_else(|| invalid(line_number, "expected a space after column name"))?;
                if name.is_empty() || data_type.trim().is_empty() {
                    return Err(invalid(line_number, "empty column name or type"));
                }
                return Ok(Column {
                    name,
                    data_type: data_type.to_owned(),
                });
            }
            '\\' => {
                let escaped = chars
                    .next()
                    .ok_or_else(|| invalid(line_number, "trailing identifier escape"))?;
                let decoded = match escaped {
                    '\\' => '\\',
                    '`' => '`',
                    '\'' => '\'',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '0' => '\0',
                    _ => return Err(invalid(line_number, "unsupported identifier escape")),
                };
                name.push(decoded);
            }
            _ => name.push(ch),
        }
    }

    Err(invalid(line_number, "unterminated column name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "columns format version: 1\n3 columns:\n`id` UInt64\n`event` Nullable(String)\n`state` Enum8('new' = 1, 'done' = 2)\n";

    #[test]
    fn parses_clickhouse_column_types_without_interpreting_them() {
        let columns = parse_columns(EXAMPLE).unwrap();
        assert_eq!(columns.len(), 3);
        assert_eq!(columns[0].name, "id");
        assert_eq!(columns[1].data_type, "Nullable(String)");
        assert_eq!(columns[2].data_type, "Enum8('new' = 1, 'done' = 2)");
    }

    #[test]
    fn supports_backtick_and_backslash_escapes() {
        let content =
            "columns format version: 1\n2 columns:\n`a\\`b` String\n`path\\\\name` String\n";
        let columns = parse_columns(content).unwrap();
        assert_eq!(columns[0].name, "a`b");
        assert_eq!(columns[1].name, "path\\name");
    }

    #[test]
    fn supports_sql_style_doubled_backticks() {
        let content = "columns format version: 1\n1 columns:\n`a``b` String\n";
        assert_eq!(parse_columns(content).unwrap()[0].name, "a`b");
    }

    #[test]
    fn rejects_invalid_headers() {
        assert!(matches!(
            parse_columns("columns format version: 2\n0 columns:\n"),
            Err(PartError::InvalidColumns { line: 1, .. })
        ));
        assert!(matches!(
            parse_columns("columns format version: 1\nwrong\n"),
            Err(PartError::InvalidColumns { line: 2, .. })
        ));
    }

    #[test]
    fn rejects_missing_and_extra_columns() {
        assert!(matches!(
            parse_columns("columns format version: 1\n2 columns:\n`a` String\n"),
            Err(PartError::InvalidColumns { line: 4, .. })
        ));
        assert!(matches!(
            parse_columns("columns format version: 1\n0 columns:\n`a` String\n"),
            Err(PartError::InvalidColumns { line: 3, .. })
        ));
    }

    #[test]
    fn rejects_broken_column_definitions() {
        for row in [
            "a String",
            "`a String",
            "`a`",
            "`` String",
            "`a` ",
            "`a\\q` String",
        ] {
            let content = format!("columns format version: 1\n1 columns:\n{row}\n");
            assert!(parse_columns(&content).is_err(), "accepted: {row:?}");
        }
    }

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
}
