//! Bounded INSERT INTO table [(columns)] SELECT ... parser.
use super::{ParseError, Parser, Select, Token, parse_select_tokens, tokenize};
use std::collections::HashSet;

const MAX_INSERT_SELECT_COLUMNS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertSelect {
    pub table: String,
    pub columns: Option<Vec<String>>,
    pub select: Select,
}

fn parse_columns(parser: &mut Parser) -> Result<Option<Vec<String>>, ParseError> {
    if !parser.take(Token::LParen) {
        return Ok(None);
    }
    let mut columns = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if columns.len() >= MAX_INSERT_SELECT_COLUMNS {
            return Err(ParseError("INSERT supports at most 64 columns".into()));
        }
        let name = parser.bounded_word()?;
        if !seen.insert(name.clone()) {
            return Err(ParseError(format!("duplicate INSERT column: {name}")));
        }
        columns.push(name);
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' after INSERT columns".into()));
    }
    Ok(Some(columns))
}

fn parse_select_suffix(parser: &mut Parser) -> Result<Option<Select>, ParseError> {
    if !parser.keyword("SELECT") {
        return Ok(None);
    }
    // The SELECT parser expects the keyword as its first token.
    parse_select_tokens(parser.tokens[parser.index - 1..].to_vec()).map(Some)
}

pub fn parse_insert_select(sql: &str) -> Result<Option<InsertSelect>, ParseError> {
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
    let table = parser.bounded_word()?;
    let columns = parse_columns(&mut parser)?;
    let Some(select) = parse_select_suffix(&mut parser)? else {
        return Ok(None);
    };
    Ok(Some(InsertSelect {
        table,
        columns,
        select,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Projection, Source};

    #[test]
    fn parses_insert_select_with_explicit_columns() {
        let parsed = parse_insert_select(
            "INSERT INTO events (id, name) SELECT id, name FROM old WHERE id > 3 LIMIT 5;",
        )
        .unwrap()
        .unwrap();
        assert_eq!(parsed.table, "events");
        assert_eq!(parsed.columns, Some(vec!["id".into(), "name".into()]));
        assert_eq!(parsed.select.source, Some(Source::Table("old".into())));
        assert_eq!(parsed.select.limit, Some(5));
        assert_eq!(
            parsed.select.projection,
            Projection::Columns(vec!["id".into(), "name".into()])
        );
        assert!(parse_insert_select("SELECT 1").unwrap().is_none());
        assert!(
            parse_insert_select("INSERT INTO events VALUES (1)")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn limits_table_and_column_identifiers_consistently() {
        let long_name = "x".repeat(129);
        assert!(parse_insert_select(&format!("INSERT INTO {long_name} SELECT 1")).is_err());
        assert!(
            parse_insert_select(&format!("INSERT INTO events ({long_name}) SELECT 1")).is_err()
        );
    }

    #[test]
    fn rejects_invalid_insert_select_forms() {
        for sql in [
            "INSERT INTO events (id, id) SELECT number FROM numbers(2)",
            "INSERT INTO events () SELECT number FROM numbers(2)",
            "INSERT INTO events SELECT number FROM numbers(2); DROP TABLE events",
            "INSERT INTO events (id) SELECT FROM numbers(2)",
            "INSERT INTO events (id,) SELECT number FROM numbers(2)",
        ] {
            assert!(parse_insert_select(sql).is_err(), "{sql}");
        }
    }
}
