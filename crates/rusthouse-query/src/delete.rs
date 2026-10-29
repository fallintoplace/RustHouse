//! DELETE FROM named_table [WHERE ...] using the existing SELECT predicate grammar.
use crate::{ParseError, Projection, Select, Source, Token, parse_select_tokens, tokenize};

fn keyword<'a>(input: &'a str, expected: &str) -> Option<&'a str> {
    let input = input.trim_start();
    let prefix = input.get(..expected.len())?;
    if !prefix.eq_ignore_ascii_case(expected) {
        return None;
    }
    let rest = input.get(expected.len()..)?;
    if rest.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
        return None;
    }
    Some(rest.trim_start())
}

fn is_supported_delete_scope(select: &Select) -> bool {
    matches!(select.source, Some(Source::Table(_)))
        && select.projection == Projection::All
        && !select.distinct
        && select.order_by.is_none()
        && select.limit.is_none()
        && select.offset.is_none()
        && select.having.is_none()
}

/// Supported only for persistent local tables. No ordering or LIMIT is accepted.
pub fn parse_delete(sql: &str) -> Result<Option<Select>, ParseError> {
    let Some(rest) = keyword(sql, "DELETE") else {
        return Ok(None);
    };
    keyword(rest, "FROM").ok_or_else(|| ParseError("DELETE requires FROM table".into()))?;
    // Work on the original tokens so a valid 65536-byte DELETE is not rejected
    // just because the SELECT projection is two bytes longer.
    let mut tokens = tokenize(sql)?;
    tokens.splice(..1, [Token::Word("SELECT".into()), Token::Star]);
    let select = parse_select_tokens(tokens)?;
    if !is_supported_delete_scope(&select) {
        return Err(ParseError(
            "DELETE supports only FROM table [WHERE predicate]".into(),
        ));
    }
    Ok(Some(select))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bounded_delete_using_select_predicates() {
        let delete = parse_delete("delete FROM events WHERE id IN (1, 3) OR name LIKE 'a%'")
            .unwrap()
            .unwrap();
        assert_eq!(delete.source, Some(Source::Table("events".into())));
        assert_eq!(delete.or_predicates.len(), 1);
        assert!(parse_delete("SELECT * FROM events").unwrap().is_none());
        assert!(parse_delete("DELETE FROM events").unwrap().is_some());
    }

    #[test]
    fn accepts_a_delete_at_the_statement_size_limit() {
        let prefix = "DELETE FROM events";
        let sql = format!("{prefix}{}", " ".repeat(64 * 1024 - prefix.len()));
        assert_eq!(
            parse_delete(&sql).unwrap().unwrap().source,
            Some(Source::Table("events".into()))
        );
        assert!(parse_delete(&(sql + " ")).is_err());
    }

    #[test]
    fn rejects_unsupported_mutation_forms() {
        for sql in [
            "DELETE events",
            "DELETE FROM",
            "DELETE FROM events WHERE",
            "DELETE FROM events ORDER BY id",
            "DELETE FROM events LIMIT 1",
            "DELETE FROM events; DROP TABLE events",
            "DELETE FROM numbers(10)",
        ] {
            assert!(parse_delete(sql).is_err(), "{sql}");
        }
    }
}
