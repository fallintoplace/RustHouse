//! Extract ClickHouse's terminal SELECT ... FORMAT name clause.
//!
//! Do not use a string search: SQL string literals and comments can contain
//! the word FORMAT, and nested expressions are not output-format directives.

pub(super) fn split_select_format(sql: &str) -> (&str, Option<&str>) {
    let trimmed = sql.trim_end();
    let statement = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    let bytes = statement.as_bytes();
    if bytes.len() <= 7
        || !statement[..6].eq_ignore_ascii_case("SELECT")
        || !bytes[6].is_ascii_whitespace()
    {
        return (sql, None);
    }

    // The only accepted grammar is a final ASCII identifier after FORMAT.
    // Anything more complex is returned to the SQL parser for rejection.
    let mut name_start = bytes.len();
    while name_start > 0
        && (bytes[name_start - 1].is_ascii_alphanumeric() || bytes[name_start - 1] == b'_')
    {
        name_start -= 1;
    }
    if name_start == bytes.len() || name_start == 0 || !bytes[name_start - 1].is_ascii_whitespace() {
        return (sql, None);
    }
    let mut keyword_end = name_start;
    while keyword_end > 0 && bytes[keyword_end - 1].is_ascii_whitespace() {
        keyword_end -= 1;
    }
    if keyword_end < 7
        || !statement[keyword_end - 6..keyword_end].eq_ignore_ascii_case("FORMAT")
        || !bytes[keyword_end - 7].is_ascii_whitespace()
    {
        return (sql, None);
    }
    let keyword_start = keyword_end - 6;
    let mut position = 0;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut line_comment = false;
    let mut block_comment = false;
    while position < keyword_start {
        let byte = bytes[position];
        let following = bytes.get(position + 1).copied();
        if line_comment {
            if byte == b'\r' || byte == b'\n' {
                line_comment = false;
            }
        } else if block_comment {
            if byte == b'*' && following == Some(b'/') {
                block_comment = false;
                position += 1;
            }
        } else if quoted {
            if byte == b'\\' {
                // ClickHouse SQL allows backslash escapes in single quoted literals.
                position += 1;
            } else if byte == b'\'' {
                if following == Some(b'\'') {
                    position += 1;
                } else {
                    quoted = false;
                }
            }
        } else if byte == b'\'' {
            quoted = true;
        } else if byte == b'-' && following == Some(b'-') {
            line_comment = true;
            position += 1;
        } else if byte == b'#' {
            line_comment = true;
        } else if byte == b'/' && following == Some(b'*') {
            block_comment = true;
            position += 1;
        } else if byte == b'(' {
            depth += 1;
        } else if byte == b')' {
            depth = depth.saturating_sub(1);
        }
        position += 1;
    }
    if quoted || line_comment || block_comment || depth != 0 {
        return (sql, None);
    }
    let prefix = statement[..keyword_start].trim_end();
    if prefix.len() <= 6 {
        return (sql, None);
    }
    (prefix, Some(&statement[name_start..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_terminal_select_formats() {
        for (sql, query, format) in [
            ("SELECT 1 FORMAT CSV", "SELECT 1", "CSV"),
            ("select number from numbers(2) format CSVWithNames;", "select number from numbers(2)", "CSVWithNames"),
            ("SELECT 1 /* formats */ FORMAT TabSeparated", "SELECT 1 /* formats */", "TabSeparated"),
            ("SELECT 1 # comment\n FORMAT JSONStringsEachRow", "SELECT 1 # comment", "JSONStringsEachRow"),
        ] {
            assert_eq!(split_select_format(sql), (query, Some(format)), "{sql}");
        }
    }

    #[test]
    fn does_not_treat_comments_literals_or_nested_functions_as_format_clauses() {
        for query in [
            "SELECT 'FORMAT CSV'",
            "SELECT 'hello' AS txt -- FORMAT CSV",
            "SELECT 1 /* FORMAT CSV */",
            "SELECT ('FORMAT CSV')",
            "SELECT 1 # FORMAT CSV",
            "SELECT 1 FORMAT",
            "SELECT 1 FORMAT CSVWithNames trailing",
            "INSERT INTO t FORMAT CSV",
            "SELECT 1 /* FORMAT CSV",
        ] {
            assert_eq!(split_select_format(query), (query, None), "{query}");
        }
    }

    #[test]
    fn supports_escaped_string_literals_and_nested_comments() {
        for query in [
            "SELECT 'it\\'s FORMAT CSV'",
            "SELECT 'FORMAT ''CSV'''",
            "SELECT 1 /* ('a' FORMAT CSV */",
            "SELECT 1 -- FORMAT CSV\n + 2",
        ] {
            assert_eq!(split_select_format(query), (query, None), "{query}");
        }
    }
}
