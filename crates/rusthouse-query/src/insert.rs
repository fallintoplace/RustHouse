//! Parse a bounded SQL INSERT ... VALUES statement without touching storage.
use super::{ParseError, Parser, Token, tokenize};
use std::collections::HashSet;

const MAX_COLUMNS: usize = 64;
const MAX_ROWS: usize = 4096;

/// A single atomic batch of literal rows destined for a catalog table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insert {
    pub table: String,
    pub columns: Option<Vec<String>>,
    pub rows: Vec<Vec<Option<String>>>,
}

/// Return None when the statement is not INSERT; reject malformed INSERTs.
pub fn parse_insert(sql: &str) -> Result<Option<Insert>, ParseError> {
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    if !parser.keyword("INSERT") {
        return Ok(None);
    }
    if !parser.keyword("INTO") {
        return Err(ParseError("INSERT requires INTO".into()));
    }
    let table = parser.word()?;
    if table.len() > 128 {
        return Err(ParseError("INSERT table name exceeds 128 bytes".into()));
    }

    let columns = if parser.take(Token::LParen) {
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        loop {
            if names.len() == MAX_COLUMNS {
                return Err(ParseError("INSERT supports at most 64 columns".into()));
            }
            let column = parser.word()?;
            if column.len() > 128 || !seen.insert(column.clone()) {
                return Err(ParseError("duplicate or oversized INSERT column".into()));
            }
            names.push(column);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after INSERT columns".into()));
        }
        Some(names)
    } else {
        None
    };

    if !parser.keyword("VALUES") {
        return Err(ParseError("INSERT requires VALUES".into()));
    }
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    loop {
        if rows.len() == MAX_ROWS {
            return Err(ParseError("INSERT supports at most 4096 rows".into()));
        }
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected '(' before INSERT row".into()));
        }
        let mut row = Vec::new();
        loop {
            if row.len() == MAX_COLUMNS {
                return Err(ParseError("INSERT supports at most 64 values per row".into()));
            }
            let value = if parser.keyword("NULL") {
                None
            } else {
                Some(parser.literal()?)
            };
            row.push(value);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after INSERT row".into()));
        }
        if let Some(first) = rows.first() {
            if row.len() != first.len() {
                return Err(ParseError("INSERT rows have different widths".into()));
            }
        }
        if columns.as_ref().is_some_and(|names| names.len() != row.len()) {
            return Err(ParseError("INSERT column count does not match VALUES".into()));
        }
        rows.push(row);
        if !parser.take(Token::Comma) {
            break;
        }
    }

    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError("unexpected tokens after INSERT statement".into()));
    }
    Ok(Some(Insert {
        table,
        columns,
        rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_literals_null_and_escaped_strings() {
        let parsed = parse_insert(
            "insert INTO events (name, id) VALUES ('O''Reilly', -5), (NULL, 7);",
        )
        .unwrap()
        .unwrap();
        assert_eq!(parsed.table, "events");
        assert_eq!(parsed.columns, Some(vec!["name".into(), "id".into()]));
        assert_eq!(
            parsed.rows,
            vec![
                vec![Some("O'Reilly".into()), Some("-5".into())],
                vec![None, Some("7".into())],
            ]
        );
        assert_eq!(
            parse_insert("INSERT INTO t VALUES ('NULL', 1)").unwrap().unwrap().rows[0][0],
            Some("NULL".into())
        );
        assert_eq!(parse_insert("SELECT * FROM events").unwrap(), None);
    }

    #[test]
    fn rejects_bad_rows_columns_and_multiple_statements() {
        for sql in [
            "INSERT events VALUES (1)",
            "INSERT INTO t SELECT 1",
            "INSERT INTO t VALUES ()",
            "INSERT INTO t VALUES (1), (2, 3)",
            "INSERT INTO t (a, a) VALUES (1, 2)",
            "INSERT INTO t (a, b) VALUES (1)",
            "INSERT INTO t VALUES (1); DROP TABLE t",
            "INSERT INTO t VALUES (true)",
            "INSERT INTO t (a,) VALUES (1)",
            "INSERT INTO t VALUES (1),",
        ] {
            assert!(parse_insert(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn keeps_unicode_and_fractional_numbers() {
        let parsed = parse_insert("INSERT INTO t VALUES ('東京', -3.25)").unwrap().unwrap();
        assert_eq!(parsed.rows[0], vec![Some("東京".into()), Some("-3.25".into())]);
    }
}
