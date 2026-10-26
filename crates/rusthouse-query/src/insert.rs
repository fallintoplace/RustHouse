//! Parser for bounded SQL INSERT ... VALUES statements.
use crate::{ParseError, Parser, Token, tokenize};
use std::collections::HashSet;

const MAX_INSERT_ROWS: usize = 4096;
const MAX_INSERT_COLUMNS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insert {
    pub table: String,
    /// Columns in the order of the VALUES tuples, if supplied.
    pub columns: Option<Vec<String>>,
    /// None denotes SQL NULL, not a string containing "NULL".
    pub rows: Vec<Vec<Option<String>>>,
}

fn identifier(parser: &mut Parser) -> Result<String, ParseError> {
    let name = parser.word()?;
    if name.len() > 128 {
        return Err(ParseError("INSERT identifiers cannot exceed 128 bytes".into()));
    }
    Ok(name)
}

/// Return None for non-INSERT SQL; reject unsupported INSERT forms and trailing statements.
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
    let table = identifier(&mut parser)?;
    let columns = if parser.take(Token::LParen) {
        let mut columns = Vec::new();
        let mut seen = HashSet::new();
        loop {
            if columns.len() == MAX_INSERT_COLUMNS {
                return Err(ParseError("INSERT supports at most 64 columns".into()));
            }
            let column = identifier(&mut parser)?;
            if !seen.insert(column.clone()) {
                return Err(ParseError(format!("duplicate INSERT column: {column}")));
            }
            columns.push(column);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after INSERT columns".into()));
        }
        Some(columns)
    } else {
        None
    };
    if !parser.keyword("VALUES") {
        return Err(ParseError("INSERT requires VALUES".into()));
    }

    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    loop {
        if rows.len() == MAX_INSERT_ROWS {
            return Err(ParseError("INSERT supports at most 4096 rows".into()));
        }
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected '(' before VALUES row".into()));
        }
        let mut row = Vec::new();
        loop {
            if row.len() == MAX_INSERT_COLUMNS {
                return Err(ParseError("INSERT supports at most 64 values per row".into()));
            }
            let value = match parser.tokens.get(parser.index) {
                Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") => {
                    parser.index += 1;
                    None
                }
                Some(Token::Number(_)) | Some(Token::String(_)) => Some(parser.literal()?),
                _ => return Err(ParseError("expected SQL literal or NULL in VALUES".into())),
            };
            row.push(value);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after VALUES row".into()));
        }
        if rows.first().is_some_and(|first| first.len() != row.len())
            || columns.as_ref().is_some_and(|names| names.len() != row.len())
        {
            return Err(ParseError("INSERT row width does not match column list".into()));
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
    fn parses_values_with_strings_nulls_and_reordered_columns() {
        let insert = parse_insert(
            "INSERT INTO events (name, id, amount) VALUES ('it''s here', -2, 3.5), ('two', 7, NULL);",
        )
        .unwrap()
        .unwrap();
        assert_eq!(insert.table, "events");
        assert_eq!(
            insert.columns,
            Some(vec!["name".into(), "id".into(), "amount".into()])
        );
        assert_eq!(
            insert.rows,
            vec![
                vec![Some("it's here".into()), Some("-2".into()), Some("3.5".into())],
                vec![Some("two".into()), Some("7".into()), None],
            ]
        );
        assert_eq!(
            parse_insert("insert into t values (NULL, 'NULL', 0)")
                .unwrap()
                .unwrap()
                .rows,
            vec![vec![None, Some("NULL".into()), Some("0".into())]]
        );
        assert!(parse_insert("SELECT 1").unwrap().is_none());
    }

    #[test]
    fn rejects_malformed_or_nonliteral_inserts() {
        for sql in [
            "INSERT t VALUES (1)",
            "INSERT INTO t SELECT 1",
            "INSERT INTO t VALUES",
            "INSERT INTO t VALUES ()",
            "INSERT INTO t VALUES (1,)",
            "INSERT INTO t VALUES (1),",
            "INSERT INTO t VALUES (1), (1, 2)",
            "INSERT INTO t (a, b) VALUES (1)",
            "INSERT INTO t (a, a) VALUES (1, 2)",
            "INSERT INTO t (a,) VALUES (1)",
            "INSERT INTO t VALUES (true)",
            "INSERT INTO t VALUES (number)",
            "INSERT INTO t VALUES (1); DROP TABLE t",
            "INSERT INTO t VALUES (1) extra",
            "INSERT INTO t VALUES (1);;",
        ] {
            assert!(parse_insert(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn rejects_bounded_width_and_statement_size() {
        let values = vec!["1"; 65].join(", ");
        assert!(parse_insert(&format!("INSERT INTO t VALUES ({values})")).is_err());
        let cols = (0..65).map(|n| format!("c{n}")).collect::<Vec<_>>().join(", ");
        assert!(parse_insert(&format!("INSERT INTO t ({cols}) VALUES (1)")).is_err());
        assert!(parse_insert(&format!("INSERT INTO t VALUES ('{}')", "x".repeat(4096))).is_err());
    }
}
