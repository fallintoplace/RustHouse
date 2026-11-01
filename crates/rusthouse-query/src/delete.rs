//! DELETE FROM named_table [WHERE ...] using the existing SELECT predicate grammar.
use crate::{ParseError, Projection, Select, Source, Token, parse_select_tokens, tokenize};

fn valid_delete_scope(select: &Select) -> bool {
    matches!(select.source, Some(Source::Table(_)))
        && select.projection == Projection::All
        && !select.distinct
        && select.rename.is_none()
        && select.alias.is_none()
        && select.order_by.is_none()
        && select.order_by_extra.is_empty()
        && select.limit.is_none()
        && select.offset.is_none()
        && select.having.is_none()
}

/// Supported only for persistent local tables. No ordering or LIMIT is accepted.
pub fn parse_delete(sql: &str) -> Result<Option<Select>, ParseError> {
    // Work on the original tokens so a valid 65536-byte DELETE is not rejected
    // just because the SELECT projection is two bytes longer.
    let mut select_tokens = tokenize(sql)?;
    if !matches!(select_tokens.first(), Some(Token::Word(word)) if word.eq_ignore_ascii_case("DELETE"))
    {
        return Ok(None);
    }
    if !matches!(select_tokens.get(1), Some(Token::Word(word)) if word.eq_ignore_ascii_case("FROM"))
    {
        return Err(ParseError("DELETE requires FROM table".into()));
    }
    select_tokens.splice(..1, [Token::Word("SELECT".into()), Token::Star]);
    let select = parse_select_tokens(select_tokens)?;
    if !valid_delete_scope(&select) {
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
            "DELETE RENAME name AS renamed FROM events",
            "DELETE RENAME id AS renamed FROM events WHERE id = 1",
        ] {
            assert!(parse_delete(sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod delete_comment_tests {
    use super::*;

    #[test]
    fn comment_before_from_is_tokenized_as_whitespace() {
        assert!(
            parse_delete("DELETE /* maintenance */ FROM data WHERE id = 1")
                .unwrap()
                .is_some()
        );
        assert!(
            parse_delete("DELETE -- maintenance\nFROM data WHERE id = 1")
                .unwrap()
                .is_some()
        );
        assert!(parse_delete("DELETE /* comment */ data").is_err());
    }
}

#[cfg(test)]
mod delete_adjacent_comment_tests {
    use super::*;

    #[test]
    fn comments_separate_delete_from_following_from_token() {
        assert!(
            parse_delete("DELETE/* tracked */FROM events WHERE id = 1")
                .unwrap()
                .is_some()
        );
        assert!(
            parse_delete("DELETE-- maintenance\nFROM events")
                .unwrap()
                .is_some()
        );
        assert!(parse_delete("DELETE/*comment*/events").is_err());
        assert!(parse_delete("DELETEX FROM events").unwrap().is_none());
    }
}

#[cfg(test)]
mod leading_delete_comment_tests {
    use super::*;

    #[test]
    fn delete_accepts_all_lexer_supported_leading_comments() {
        for prefix in [
            "/* explanation */ ",
            "-- line comment\n",
            "# comment\r\n",
            "/* outer /* inner */ end */\n",
        ] {
            let sql = format!("{prefix}DELETE FROM events WHERE id = 42");
            let result = parse_delete(&sql).unwrap().unwrap();
            assert_eq!(result.source, Some(Source::Table("events".into())));
            assert_eq!(result.projection, Projection::All);
        }
    }

    #[test]
    fn non_delete_statements_remain_unrecognized() {
        assert!(
            parse_delete("/* comment */ SELECT * FROM events")
                .unwrap()
                .is_none()
        );
        assert!(parse_delete("-- comment only").unwrap().is_none());
    }
}

#[cfg(test)]
mod delete_projection_guard_tests {
    use super::*;

    #[test]
    fn mutation_parser_rejects_select_only_projection_clauses() {
        for sql in [
            "DELETE RENAME id AS renamed FROM events",
            "DELETE RENAME id AS renamed FROM events WHERE id = 2",
        ] {
            assert!(parse_delete(sql).is_err(), "{sql}");
        }
        assert!(
            parse_delete("DELETE FROM events WHERE id = 2")
                .unwrap()
                .is_some()
        );
    }
}
