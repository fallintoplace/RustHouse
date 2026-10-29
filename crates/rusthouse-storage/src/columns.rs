use crate::{Column, PartError};
use std::collections::HashSet;

const MAX_INITIAL_COLUMNS: usize = 1024;

pub fn parse_columns(content: &str) -> Result<Vec<Column>, PartError> {
    let mut lines = content.lines();
    if lines.next() != Some("columns format version: 1") {
        return Err(invalid(1, "expected columns format version: 1"));
    }

    let header = lines
        .next()
        .ok_or_else(|| invalid(2, "missing column-count header"))?;
    let expected =
        parse_column_count(header).ok_or_else(|| invalid(2, "expected '<count> columns:'"))?;

    // Keep malformed huge counts from triggering an unbounded allocation.
    let capacity = expected.min(MAX_INITIAL_COLUMNS);
    let mut columns = Vec::with_capacity(capacity);
    let mut names = HashSet::with_capacity(capacity);
    for index in 0..expected {
        let line_number = index + 3;
        let line = lines
            .next()
            .ok_or_else(|| invalid(line_number, "fewer columns than declared"))?;
        let column = parse_column(line, line_number)?;
        if !names.insert(column.name.clone()) {
            return Err(invalid(line_number, "duplicate column name"));
        }
        columns.push(column);
    }
    if lines.next().is_some() {
        return Err(invalid(expected + 3, "more columns than declared"));
    }
    if !content.ends_with('\n') {
        return Err(invalid(expected + 2, "missing final newline"));
    }

    Ok(columns)
}

fn parse_column_count(header: &str) -> Option<usize> {
    header
        .strip_suffix(" columns:")
        .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|value| value.parse::<usize>().ok())
}

fn invalid(line: usize, message: &'static str) -> PartError {
    PartError::InvalidColumns { line, message }
}

fn hex_identifier_digit(
    chars: &mut std::str::Chars<'_>,
    line_number: usize,
) -> Result<u32, PartError> {
    chars
        .next()
        .and_then(|digit| digit.to_digit(16))
        .ok_or_else(|| invalid(line_number, "invalid hex identifier escape"))
}

fn append_utf8_character(name: &mut Vec<u8>, character: char) {
    let mut encoded = [0; 4];
    name.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
}

fn append_identifier_escape(
    chars: &mut std::str::Chars<'_>,
    name: &mut Vec<u8>,
    line_number: usize,
) -> Result<(), PartError> {
    let escaped = chars
        .next()
        .ok_or_else(|| invalid(line_number, "trailing identifier escape"))?;
    let decoded = match escaped {
        'x' => {
            let high = hex_identifier_digit(chars, line_number)?;
            let low = hex_identifier_digit(chars, line_number)?;
            (high * 16 + low) as u8
        }
        'N' => return Ok(()),
        '\\' => b'\\',
        '`' => b'`',
        '\'' => b'\'',
        '"' => b'"',
        '/' => b'/',
        '=' => b'=',
        'n' => b'\n',
        'r' => b'\r',
        't' => b'\t',
        '0' => b'\0',
        'a' => b'\x07',
        'b' => b'\x08',
        'e' => b'\x1b',
        'f' => b'\x0c',
        'v' => b'\x0b',
        _ => {
            name.push(b'\\');
            append_utf8_character(name, escaped);
            return Ok(());
        }
    };
    name.push(decoded);
    Ok(())
}

fn finish_column(name: Vec<u8>, data_type: &str, line_number: usize) -> Result<Column, PartError> {
    if name.is_empty() || data_type.is_empty() || data_type.trim() != data_type {
        return Err(invalid(line_number, "empty column name or invalid type"));
    }
    let name = String::from_utf8(name)
        .map_err(|_| invalid(line_number, "column name is not valid UTF-8"))?;
    Ok(Column {
        name,
        data_type: data_type.to_owned(),
    })
}

fn parse_column(line: &str, line_number: usize) -> Result<Column, PartError> {
    let mut chars = line
        .strip_prefix('`')
        .ok_or_else(|| invalid(line_number, "expected a backtick-quoted column name"))?
        .chars();
    let mut name = Vec::new();

    while let Some(ch) = chars.next() {
        match ch {
            '`' if chars.as_str().starts_with('`') => {
                chars.next();
                name.push(b'`');
            }
            '`' => {
                let data_type = chars
                    .as_str()
                    .strip_prefix(' ')
                    .ok_or_else(|| invalid(line_number, "expected a space after column name"))?;
                return finish_column(name, data_type, line_number);
            }
            '\\' => append_identifier_escape(&mut chars, &mut name, line_number)?,
            _ => {
                append_utf8_character(&mut name, ch);
            }
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
    fn supports_hex_and_control_escapes() {
        let content =
            "columns format version: 1\n2 columns:\n`caf\\xC3\\xA9` String\n`bell\\a` UInt8\n";
        let columns = parse_columns(content).unwrap();
        assert_eq!(columns[0].name, "café");
        assert_eq!(columns[1].name, "bell\u{7}");
    }

    #[test]
    fn rejects_invalid_hex_and_non_utf8_names() {
        for column in [
            r"`a\x` String",
            r"`a\x0` String",
            r"`a\xQ1` String",
            r"`a\xFF` String",
        ] {
            let content = format!("columns format version: 1\n1 columns:\n{column}\n");
            assert!(parse_columns(&content).is_err(), "accepted: {column:?}");
        }
    }

    #[test]
    fn rejects_extra_whitespace_in_column_types() {
        for column in ["`a`  String", "`a` String ", "`a` \t"] {
            let content = format!("columns format version: 1\n1 columns:\n{column}\n");
            assert!(parse_columns(&content).is_err(), "accepted: {column:?}");
        }
    }

    #[test]
    fn supports_clickhouse_compatible_escapes() {
        let columns = parse_columns(
            "columns format version: 1\n3 columns:\n`unknown\\q` String\n`equal\\=slash\\/quote\\\"` UInt8\n`a\\Nb` UInt16\n",
        )
        .unwrap();

        assert_eq!(columns[0].name, r"unknown\q");
        assert_eq!(columns[1].name, "equal=slash/quote\"");
        assert_eq!(columns[2].name, "ab");
    }

    #[test]
    fn rejects_truncated_last_line() {
        for content in [
            "columns format version: 1\n0 columns:",
            "columns format version: 1\n1 columns:\n`id` UInt64",
        ] {
            assert!(matches!(
                parse_columns(content),
                Err(PartError::InvalidColumns {
                    message: "missing final newline",
                    ..
                })
            ));
        }
    }

    #[test]
    fn supports_empty_column_lists_and_unicode_identifiers() {
        assert!(
            parse_columns("columns format version: 1\n0 columns:\n")
                .unwrap()
                .is_empty()
        );

        let columns =
            parse_columns("columns format version: 1\n1 columns:\n`測試` String\n").unwrap();
        assert_eq!(columns[0].name, "測試");
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
        for header in ["+1 columns:", "-1 columns:", " 1 columns:", "1 columns: "] {
            let content = format!("columns format version: 1\n{header}\n`id` UInt64\n");
            assert!(
                matches!(
                    parse_columns(&content),
                    Err(PartError::InvalidColumns { line: 2, .. })
                ),
                "{header:?}"
            );
        }
    }

    #[test]
    fn rejects_duplicate_decoded_names() {
        for text in [
            "columns format version: 1\n2 columns:\n`id` UInt32\n`id` UInt64\n",
            "columns format version: 1\n2 columns:\n`a` UInt8\n`\\x61` UInt16\n",
        ] {
            assert!(matches!(
                parse_columns(text),
                Err(PartError::InvalidColumns {
                    line: 4,
                    message: "duplicate column name"
                })
            ));
        }
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
        for row in ["a String", "`a String", "`a`", "`` String", "`a` "] {
            let content = format!("columns format version: 1\n1 columns:\n{row}\n");
            assert!(parse_columns(&content).is_err(), "accepted: {row:?}");
        }
    }
}
