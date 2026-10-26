//! Bounded DELETE statement parser shared by local database mode.
use super::{ParseError, Parser, Predicate, Token, parse_where, tokenize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delete {
    pub table: String,
    pub predicates: Vec<Predicate>,
    pub or_predicates: Vec<Vec<Predicate>>,
}

pub fn parse_delete(sql: &str) -> Result<Option<Delete>, ParseError> {
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    if !parser.keyword("DELETE") {
        return Ok(None);
    }
    if !parser.keyword("FROM") {
        return Err(ParseError("DELETE requires FROM table".into()));
    }
    let table = parser.word()?;
    if table.len() > 128 {
        return Err(ParseError("DELETE table name exceeds 128 bytes".into()));
    }
    let (predicates, or_predicates) = parse_where(&mut parser)?;
    if predicates.is_empty() {
        return Err(ParseError("DELETE requires WHERE; use TRUNCATE TABLE to delete all rows".into()));
    }
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError("unexpected tokens after DELETE".into()));
    }
    Ok(Some(Delete {
        table,
        predicates,
        or_predicates,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_delete_with_shared_predicates() {
        let delete = parse_delete("DELETE FROM events WHERE id >= 2 AND note IS NOT NULL;")
            .unwrap()
            .unwrap();
        assert_eq!(delete.table, "events");
        assert_eq!(delete.predicates.len(), 2);
        assert!(delete.or_predicates.is_empty());
        let branches = parse_delete("DELETE FROM events WHERE name = 'WHERE' OR id IN (1, 2)")
            .unwrap()
            .unwrap();
        assert_eq!(branches.or_predicates.len(), 1);
    }

    #[test]
    fn refuses_unbounded_or_malformed_deletes() {
        for sql in [
            "DELETE events WHERE id = 1",
            "DELETE FROM events",
            "DELETE FROM events WHERE",
            "DELETE FROM events WHERE id = 1 LIMIT 1",
            "DELETE FROM events WHERE id = 1; DROP TABLE events",
        ] {
            assert!(parse_delete(sql).is_err(), "{sql}");
        }
        assert!(parse_delete("SELECT * FROM events").unwrap().is_none());
    }
}
