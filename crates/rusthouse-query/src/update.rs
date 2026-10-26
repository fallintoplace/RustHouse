//! Literal UPDATE assignments with bounded SELECT-style WHERE predicates.
use super::{Operator, ParseError, Parser, Predicate, Token, parse_where, tokenize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub column: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub table: String,
    pub assignments: Vec<Assignment>,
    pub predicates: Vec<Predicate>,
    pub or_predicates: Vec<Vec<Predicate>>,
}

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
        return Err(ParseError("UPDATE table name exceeds 128 bytes".into()));
    }
    if !parser.keyword("SET") {
        return Err(ParseError("UPDATE requires SET".into()));
    }
    let mut assignments = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if assignments.len() == 64 {
            return Err(ParseError("UPDATE supports at most 64 assignments".into()));
        }
        let column = parser.word()?;
        if !seen.insert(column.clone()) {
            return Err(ParseError(format!("duplicate UPDATE column: {column}")));
        }
        if !parser.take(Token::Operator(Operator::Eq)) {
            return Err(ParseError("UPDATE assignments require =".into()));
        }
        let value = if parser.keyword("NULL") {
            None
        } else {
            Some(parser.literal()?)
        };
        assignments.push(Assignment { column, value });
        if !parser.take(Token::Comma) {
            break;
        }
    }
    let (predicates, or_predicates) = parse_where(&mut parser)?;
    if predicates.is_empty() {
        return Err(ParseError(
            "UPDATE requires WHERE to prevent accidental full-table changes".into(),
        ));
    }
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError("unexpected tokens after UPDATE".into()));
    }
    Ok(Some(Update {
        table,
        assignments,
        predicates,
        or_predicates,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_assignments_and_filters() {
        let result = parse_update(
            "UPDATE events SET name = 'Ada', note = NULL WHERE id IN (1, 2) OR id = 4;",
        )
        .unwrap()
        .unwrap();
        assert_eq!(result.table, "events");
        assert_eq!(
            result.assignments,
            vec![
                Assignment {
                    column: "name".into(),
                    value: Some("Ada".into()),
                },
                Assignment {
                    column: "note".into(),
                    value: None,
                },
            ]
        );
        assert_eq!(result.or_predicates.len(), 1);
    }

    #[test]
    fn rejects_unbounded_and_invalid_updates() {
        for query in [
            "UPDATE events SET name = 'oops'",
            "UPDATE events name = 'oops' WHERE id = 1",
            "UPDATE events SET id = 1, id = 2 WHERE id = 3",
            "UPDATE events SET name = lower(name) WHERE id = 1",
            "UPDATE events SET name = 'hi' WHERE id = 1 LIMIT 1",
            "UPDATE events SET name = 'hi' WHERE id = 1; DROP TABLE events",
        ] {
            assert!(parse_update(query).is_err(), "{query}");
        }
        assert!(parse_update("SELECT * FROM events").unwrap().is_none());
    }
}
