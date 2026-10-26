//! Bounded UPDATE table SET column = literal [WHERE ...] parser.
use crate::{Operator, ParseError, Parser, Select, Source, Token, parse_select, tokenize};
use std::collections::HashSet;

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
        _ => Err(ParseError(
            "UPDATE SET accepts only literal values or NULL".into(),
        )),
    }
}

fn token_sql(token: &Token) -> String {
    match token {
        Token::Word(value) | Token::Number(value) => value.clone(),
        Token::String(value) => format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''")),
        Token::Star => "*".into(),
        Token::Percent => "%".into(),
        Token::Comma => ",".into(),
        Token::LParen => "(".into(),
        Token::RParen => ")".into(),
        Token::Semicolon => ";".into(),
        Token::Operator(op) => match op {
            Operator::Eq => "=",
            Operator::Ne => "!=",
            Operator::Lt => "<",
            Operator::Le => "<=",
            Operator::Gt => ">",
            Operator::Ge => ">=",
            Operator::Modulo(_) | Operator::ModuloNe(_) => "%",
        }
        .into(),
    }
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
    if table.len() > 128 {
        return Err(ParseError("UPDATE table name is too long".into()));
    }
    if !parser.keyword("SET") {
        return Err(ParseError("UPDATE requires SET assignments".into()));
    }
    let mut assignments = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if assignments.len() >= 64 {
            return Err(ParseError("UPDATE supports at most 64 columns".into()));
        }
        let column = parser.word()?;
        if !seen.insert(column.clone()) {
            return Err(ParseError(format!("duplicate UPDATE column: {column}")));
        }
        if !parser.take(Token::Operator(Operator::Eq)) {
            return Err(ParseError("UPDATE assignments require =".into()));
        }
        assignments.push((column, value(&mut parser)?));
        if !parser.take(Token::Comma) {
            break;
        }
    }

    let mut query = format!("SELECT * FROM {table}");
    if parser.keyword("WHERE") {
        if parser.index == parser.tokens.len() {
            return Err(ParseError("UPDATE WHERE requires a predicate".into()));
        }
        query.push_str(" WHERE ");
        query.push_str(
            &parser.tokens[parser.index..]
                .iter()
                .map(token_sql)
                .collect::<Vec<_>>()
                .join(" "),
        );
    } else {
        parser.take(Token::Semicolon);
        if parser.index != parser.tokens.len() {
            return Err(ParseError(
                "unexpected tokens after UPDATE assignment".into(),
            ));
        }
    }
    let select = parse_select(&query)?;
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
}
