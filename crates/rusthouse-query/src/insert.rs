//! Deliberately narrow INSERT ... VALUES parser for local writable tables.
use super::{ParseError, Parser, Token, tokenize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insert {
    pub table: String,
    pub columns: Option<Vec<String>>,
    pub rows: Vec<Vec<Option<String>>>,
}

fn identifier(parser: &mut Parser) -> Result<String, ParseError> {
    let name = parser.word()?;
    if name.len() > 128 {
        return Err(ParseError("identifiers cannot exceed 128 bytes".into()));
    }
    Ok(name)
}

fn value(parser: &mut Parser) -> Result<Option<String>, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::String(value)) | Some(Token::Number(value)) => {
            parser.index += 1;
            Ok(Some(value.clone()))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") => {
            parser.index += 1;
            Ok(None)
        }
        _ => Err(ParseError("INSERT values must be literals or NULL".into())),
    }
}

/// Recognize INSERT INTO table [(column, ...)] VALUES (...), (...).
/// Only literal rows are accepted, with no expressions or implicit casts.
pub fn parse_insert(sql: &str) -> Result<Option<Insert>, ParseError> {
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    if !parser.keyword("INSERT") {
        return Ok(None);
    }
    if !parser.keyword("INTO") {
        return Err(ParseError("expected INTO after INSERT".into()));
    }
    let table = identifier(&mut parser)?;
    let columns = if parser.take(Token::LParen) {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        loop {
            if result.len() >= 64 {
                return Err(ParseError("INSERT supports at most 64 columns".into()));
            }
            let column = identifier(&mut parser)?;
            if !seen.insert(column.clone()) {
                return Err(ParseError(format!("duplicate INSERT column: {column}")));
            }
            result.push(column);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after INSERT columns".into()));
        }
        Some(result)
    } else {
        None
    };
    if !parser.keyword("VALUES") {
        return Err(ParseError("expected VALUES after INSERT table".into()));
    }
    let mut rows = Vec::new();
    loop {
        if rows.len() >= 4096 {
            return Err(ParseError("INSERT supports at most 4096 rows".into()));
        }
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected '(' after VALUES".into()));
        }
        let mut row = Vec::new();
        loop {
            if row.len() >= 64 {
                return Err(ParseError(
                    "INSERT supports at most 64 values per row".into(),
                ));
            }
            row.push(value(&mut parser)?);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after row values".into()));
        }
        if let Some(names) = &columns {
            if row.len() != names.len() {
                return Err(ParseError(
                    "INSERT row width does not match column list".into(),
                ));
            }
        } else if rows
            .first()
            .is_some_and(|previous: &Vec<Option<String>>| previous.len() != row.len())
        {
            return Err(ParseError("INSERT rows must have equal width".into()));
        }
        rows.push(row);
        if !parser.take(Token::Comma) {
            break;
        }
    }
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected tokens after INSERT statement".into(),
        ));
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
    fn parses_named_multirow_insert_and_escaped_literals() {
        assert_eq!(
            parse_insert("INSERT INTO events (name, id) VALUES ('O''Brien', 1), ('two', 2);")
                .unwrap(),
            Some(Insert {
                table: "events".into(),
                columns: Some(vec!["name".into(), "id".into()]),
                rows: vec![
                    vec![Some("O'Brien".into()), Some("1".into())],
                    vec![Some("two".into()), Some("2".into())],
                ],
            })
        );
        assert_eq!(
            parse_insert("insert into events values (-2, null)")
                .unwrap()
                .unwrap()
                .rows,
            vec![vec![Some("-2".into()), None]]
        );
        assert!(parse_insert("SELECT 1").unwrap().is_none());
    }

    #[test]
    fn rejects_expressions_duplicate_columns_and_multiple_statements() {
        for input in [
            "INSERT events VALUES (1)",
            "INSERT INTO events VALUES ()",
            "INSERT INTO events VALUES (1), (2, 3)",
            "INSERT INTO events VALUES (1 + 2)",
            "INSERT INTO events VALUES (NOW())",
            "INSERT INTO events VALUES (one)",
            "INSERT INTO events (id, id) VALUES (1, 2)",
            "INSERT INTO events (id, name) VALUES (1)",
            "INSERT INTO events VALUES (1); DELETE FROM events",
        ] {
            assert!(parse_insert(input).is_err(), "{input}");
        }
    }
}
