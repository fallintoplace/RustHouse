//! Bounded UPDATE table SET column = literal [WHERE ...] parser.
use crate::{Operator, ParseError, Parser, Select, Source, Token, parse_select_tokens, tokenize};
use std::collections::HashSet;

const MAX_UPDATE_ASSIGNMENTS: usize = 64;
const MAX_UPDATE_TABLE_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub table: String,
    pub assignments: Vec<(String, Option<String>)>,
    pub select: Select,
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
        Some(Token::Plus) => {
            parser.index += 1;
            match parser.tokens.get(parser.index) {
                Some(Token::Number(value)) if !value.starts_with('-') => {
                    let value = value.clone();
                    parser.index += 1;
                    Ok(Some(value))
                }
                _ => Err(ParseError("UPDATE + requires a numeric literal".into())),
            }
        }
        _ => Err(ParseError(
            "UPDATE SET accepts only literal values or NULL".into(),
        )),
    }
}

fn parse_assignments(parser: &mut Parser) -> Result<Vec<(String, Option<String>)>, ParseError> {
    let mut assignments = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if assignments.len() >= MAX_UPDATE_ASSIGNMENTS {
            return Err(ParseError("UPDATE supports at most 64 columns".into()));
        }
        let column = parser.bounded_word()?;
        if !seen.insert(column.clone()) {
            return Err(ParseError(format!("duplicate UPDATE column: {column}")));
        }
        if !parser.take(Token::Operator(Operator::Eq)) {
            return Err(ParseError("UPDATE assignments require =".into()));
        }
        assignments.push((column, value(parser)?));
        if !parser.take(Token::Comma) {
            break;
        }
    }
    Ok(assignments)
}

fn update_select_tokens(parser: &mut Parser, table: &str) -> Result<Vec<Token>, ParseError> {
    let mut tokens = vec![
        Token::Word("SELECT".into()),
        Token::Star,
        Token::Word("FROM".into()),
        Token::Word(table.into()),
    ];
    if parser.keyword("WHERE") {
        if parser.index == parser.tokens.len() {
            return Err(ParseError("UPDATE WHERE requires a predicate".into()));
        }
        tokens.push(Token::Word("WHERE".into()));
        tokens.extend_from_slice(&parser.tokens[parser.index..]);
    } else {
        parser.take(Token::Semicolon);
        if parser.index != parser.tokens.len() {
            return Err(ParseError(
                "unexpected tokens after UPDATE assignment".into(),
            ));
        }
    }
    Ok(tokens)
}

fn validate_update_scope(select: &Select) -> Result<(), ParseError> {
    if !matches!(select.source, Some(Source::Table(_)))
        || select.limit.is_some()
        || select.offset.is_some()
        || select.order_by.is_some()
        || select.having.is_some()
    {
        return Err(ParseError(
            "UPDATE supports only table SET assignments [WHERE predicate]".into(),
        ));
    }
    Ok(())
}

/// Rewrite predicates using the SELECT parser to keep NULL, LIKE and OR semantics shared.
pub fn parse_update(sql: &str) -> Result<Option<Update>, ParseError> {
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    if !parser.keyword("UPDATE") {
        return Ok(None);
    }
    let table = parser.word()?;
    if table.len() > MAX_UPDATE_TABLE_BYTES {
        return Err(ParseError("UPDATE table name is too long".into()));
    }
    if !parser.keyword("SET") {
        return Err(ParseError("UPDATE requires SET assignments".into()));
    }
    let assignments = parse_assignments(&mut parser)?;

    let tokens = update_select_tokens(&mut parser, &table)?;
    let select = parse_select_tokens(tokens)?;
    validate_update_scope(&select)?;
    Ok(Some(Update {
        table,
        assignments,
        select,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuses_where_tokens_with_escaped_literals() {
        let sql = r"UPDATE events SET id = 1 WHERE name = 'it''s\\back' AND id != -2";
        let mut parser = Parser {
            tokens: tokenize(sql).unwrap(),
            index: 0,
        };
        assert!(parser.keyword("UPDATE"));
        assert_eq!(parser.word().unwrap(), "events");
        assert!(parser.keyword("SET"));
        parse_assignments(&mut parser).unwrap();
        let selected = update_select_tokens(&mut parser, "events").unwrap();
        assert!(selected.contains(&Token::String("it's\\back".into())));
        let update = parse_update(sql).unwrap().unwrap();
        assert_eq!(update.select.predicates.len(), 2);
    }

    #[test]
    fn parses_nullable_updates_and_filters() {
        let update =
            parse_update("UPDATE events SET name = 'O''Brien', value = NULL WHERE id IN (1, 3)")
                .unwrap()
                .unwrap();
        assert_eq!(update.table, "events");
        assert_eq!(
            update.assignments,
            vec![
                ("name".into(), Some("O'Brien".into())),
                ("value".into(), None),
            ]
        );
        assert_eq!(update.select.source, Some(Source::Table("events".into())));
        assert!(parse_update("SELECT 1").unwrap().is_none());
        assert!(
            parse_update("UPDATE events SET id = -12;")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn forwards_where_predicates_with_statement_terminators() {
        let update = parse_update("UPDATE events SET id = 1 WHERE id >= 1;")
            .unwrap()
            .unwrap();
        assert_eq!(update.select.source, Some(Source::Table("events".into())));
    }

    #[test]
    fn rejects_unsafe_and_unsupported_update_forms() {
        for statement in [
            "UPDATE events",
            "UPDATE events SET",
            "UPDATE events SET id 5",
            "UPDATE events SET id = id + 1",
            "UPDATE events SET id = 1, id = 2",
            "UPDATE events SET id = 1 WHERE",
            "UPDATE events SET id = 1 WHERE id = 1 LIMIT 1",
            "UPDATE events SET id = 1; DROP TABLE events",
        ] {
            assert!(parse_update(statement).is_err(), "{statement}");
        }
    }
    #[test]
    fn restricts_update_column_identifiers_to_128_bytes() {
        let accepted = "a".repeat(128);
        let rejected = "a".repeat(129);
        assert!(parse_update(&format!("UPDATE events SET {accepted} = 1")).is_ok());
        assert!(parse_update(&format!("UPDATE events SET {rejected} = 1")).is_err());
    }

    #[test]
    fn accepts_unsigned_numeric_literals_with_unary_plus() {
        let parsed = parse_update("UPDATE events SET id = +3 WHERE id = 1")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.assignments, vec![("id".into(), Some("3".into()))]);
        assert!(parse_update("UPDATE events SET id = +NULL").is_err());
    }
}
