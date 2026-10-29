//! Extract the optional top-level ClickHouse SQL `FORMAT name` clause.
//! Only a trailing FORMAT clause is recognized. The bounded scanner respects
//! strings, comments and parentheses instead of slicing SQL at a text match.

use super::QueryFormat;

#[derive(Clone, Copy)]
enum Piece {
    Word(usize, usize),
    Other,
    Semicolon,
}

pub(super) fn extract(sql: &str) -> Result<(&str, Option<QueryFormat>), &'static str> {
    let bytes = sql.as_bytes();
    let mut pieces = Vec::new();
    let mut offset = 0;
    let mut parens = 0usize;
    while offset < bytes.len() {
        match bytes[offset] {
            b' ' | b'\t' | b'\r' | b'\n' => offset += 1,
            b'#' => {
                while offset < bytes.len() && bytes[offset] != b'\n' {
                    offset += 1;
                }
            }
            b'-' if bytes.get(offset + 1) == Some(&b'-') => {
                offset += 2;
                while offset < bytes.len() && bytes[offset] != b'\n' {
                    offset += 1;
                }
            }
            b'/' if bytes.get(offset + 1) == Some(&b'*') => {
                offset += 2;
                let mut closed = false;
                while offset + 1 < bytes.len() {
                    if bytes[offset] == b'*' && bytes[offset + 1] == b'/' {
                        offset += 2;
                        closed = true;
                        break;
                    }
                    offset += 1;
                }
                if !closed {
                    return Err("unterminated SQL block comment");
                }
            }
            b'\'' => {
                if parens == 0 {
                    pieces.push(Piece::Other);
                }
                offset += 1;
                let mut closed = false;
                while offset < bytes.len() {
                    if bytes[offset] == b'\\'
                        && matches!(bytes.get(offset + 1), Some(b'\'' | b'\\'))
                    {
                        offset += 2;
                    } else if bytes[offset] == b'\''
                        && bytes.get(offset + 1) == Some(&b'\'')
                    {
                        offset += 2;
                    } else if bytes[offset] == b'\'' {
                        offset += 1;
                        closed = true;
                        break;
                    } else {
                        offset += 1;
                    }
                }
                if !closed {
                    return Err("unterminated SQL string literal");
                }
            }
            b'(' => {
                if parens == 0 {
                    pieces.push(Piece::Other);
                }
                parens += 1;
                offset += 1;
            }
            b')' => {
                if parens == 0 {
                    pieces.push(Piece::Other);
                } else {
                    parens -= 1;
                    if parens == 0 {
                        pieces.push(Piece::Other);
                    }
                }
                offset += 1;
            }
            byte if byte.is_ascii_alphanumeric() || byte == b'_' => {
                let start = offset;
                offset += 1;
                while bytes
                    .get(offset)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    offset += 1;
                }
                if parens == 0 {
                    pieces.push(Piece::Word(start, offset));
                }
            }
            b';' => {
                if parens == 0 {
                    pieces.push(Piece::Semicolon);
                }
                offset += 1;
            }
            _ => {
                if parens == 0 {
                    pieces.push(Piece::Other);
                }
                offset += 1;
            }
        }
    }

    let trailing = if matches!(pieces.last(), Some(Piece::Semicolon)) {
        &pieces[..pieces.len() - 1]
    } else {
        &pieces[..]
    };
    if let [.., Piece::Word(keyword_start, keyword_end), Piece::Word(name_start, name_end)] =
        trailing
    {
        if sql[*keyword_start..*keyword_end].eq_ignore_ascii_case("FORMAT")
            && *keyword_end < *name_start
            && sql[*keyword_end..*name_start]
                .bytes()
                .all(|byte| byte.is_ascii_whitespace())
        {
            let statement = sql[..*keyword_start].trim_end();
            if statement.is_empty() {
                return Err("FORMAT requires a SELECT statement");
            }
            return Ok((
                statement,
                Some(QueryFormat::parse(&sql[*name_start..*name_end])?),
            ));
        }
    }
    Ok((sql, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_terminal_sql_format_only() {
        for sql in [
            "SELECT 1 FORMAT CSV",
            "SELECT 1\nFORMAT CSV;",
            "SELECT 'x FORMAT TSV' FORMAT CSV -- comment",
            "SELECT 1 /* FORMAT TSV */ FORMAT CSV",
        ] {
            let (statement, format) = extract(sql).unwrap();
            assert!(statement.starts_with("SELECT"));
            assert!(matches!(format, Some(QueryFormat::CsvNoHeader)));
        }
    }

    #[test]
    fn leaves_quoted_nested_and_incomplete_syntax_to_sql_parser() {
        for sql in [
            "SELECT 'FORMAT CSV'",
            "SELECT (1) AS format",
            "SELECT 1 FORMAT",
            "SELECT 'FORMAT CSV' AS label",
            "SELECT 1 /* FORMAT CSV */",
            "SELECT 1 FORMAT CSV EXTRA",
        ] {
            assert!(extract(sql).unwrap().1.is_none(), "{sql}");
        }
    }

    #[test]
    fn fails_closed_for_unknown_formats_and_unclosed_literals() {
        assert!(extract("SELECT 1 FORMAT JSONEachRow").is_err());
        assert!(extract("SELECT 1 FORMAT CSV /* unfinished").is_err());
        assert!(extract("SELECT 'unterminated FORMAT TSV").is_err());
    }
}
