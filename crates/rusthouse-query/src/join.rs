//! Bounded two-table equi-join syntax for persistent SQL tables.
use super::{ParseError, Parser, Token, tokenize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinColumn {
    pub relation: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinSelect {
    pub left: String,
    pub right: String,
    pub left_alias: String,
    pub right_alias: String,
    pub kind: JoinType,
    /// None projects every column from both tables.
    pub columns: Option<Vec<JoinColumn>>,
    pub keys: Vec<(JoinColumn, JoinColumn)>,
    pub order_by: Option<(JoinColumn, bool)>,
    pub limit: Option<usize>,
    pub offset: usize,
}

fn qualified(parser: &mut Parser) -> Result<JoinColumn, ParseError> {
    let relation = parser.word()?;
    if !parser.take(Token::Dot) {
        return Err(ParseError(
            "JOIN columns must be qualified (alias.column)".into(),
        ));
    }
    let name = parser.word()?;
    Ok(JoinColumn { relation, name })
}

fn alias(parser: &mut Parser, table: &str) -> Result<String, ParseError> {
    if parser.keyword("AS") {
        return parser.word();
    }
    match parser.tokens.get(parser.index) {
        Some(Token::Word(word))
            if ![
                "INNER", "LEFT", "RIGHT", "FULL", "CROSS", "OUTER", "JOIN", "ON", "WHERE", "GROUP",
                "ORDER", "LIMIT", "OFFSET",
            ]
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword)) =>
        {
            parser.word()
        }
        _ => Ok(table.to_owned()),
    }
}

fn bound(parser: &mut Parser, label: &str) -> Result<usize, ParseError> {
    let number = parser.number()?;
    if !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ParseError(format!(
            "{label} must be a non-negative integer"
        )));
    }
    number
        .parse::<usize>()
        .map_err(|_| ParseError(format!("{label} exceeds address space")))
}

/// Return None for queries without a JOIN keyword after FROM.
pub fn parse_join(sql: &str) -> Result<Option<JoinSelect>, ParseError> {
    let tokens = tokenize(sql)?;
    if !matches!(tokens.first(), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT")) {
        return Ok(None);
    }
    let Some(from) = tokens
        .iter()
        .position(|token| matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("FROM")))
    else {
        return Ok(None);
    };
    if !tokens
        .iter()
        .skip(from + 1)
        .any(|token| matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("JOIN")))
    {
        return Ok(None);
    }
    let mut parser = Parser { tokens, index: 0 };
    parser.keyword("SELECT");
    let columns = if parser.take(Token::Star) {
        None
    } else {
        let mut columns = Vec::new();
        loop {
            if columns.len() == 64 {
                return Err(ParseError(
                    "JOIN supports at most 64 projected columns".into(),
                ));
            }
            columns.push(qualified(&mut parser)?);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        Some(columns)
    };
    if !parser.keyword("FROM") {
        return Err(ParseError("JOIN requires FROM".into()));
    }
    let left = parser.word()?;
    let left_alias = alias(&mut parser, &left)?;
    if left.len() > 128 || left_alias.len() > 128 {
        return Err(ParseError(
            "table identifiers cannot exceed 128 bytes".into(),
        ));
    }
    let kind = if parser.keyword("LEFT") {
        JoinType::Left
    } else if parser.keyword("RIGHT") {
        JoinType::Right
    } else if parser.keyword("FULL") {
        JoinType::Full
    } else {
        parser.keyword("INNER");
        JoinType::Inner
    };
    if !matches!(kind, JoinType::Inner) {
        parser.keyword("OUTER");
    }
    if !parser.keyword("JOIN") {
        return Err(ParseError("expected JOIN after join type".into()));
    }
    let right = parser.word()?;
    let right_alias = alias(&mut parser, &right)?;
    if right.len() > 128 || right_alias.len() > 128 {
        return Err(ParseError(
            "table identifiers cannot exceed 128 bytes".into(),
        ));
    }
    if left_alias == right_alias {
        return Err(ParseError("joined tables require distinct aliases".into()));
    }
    if !parser.keyword("ON") {
        return Err(ParseError("JOIN requires ON".into()));
    }
    let first = qualified(&mut parser)?;
    if !parser.take(Token::Operator(super::Operator::Eq)) {
        return Err(ParseError(
            "JOIN ON currently supports only equality".into(),
        ));
    }
    let second = qualified(&mut parser)?;
    let order_by = if parser.keyword("ORDER") {
        if !parser.keyword("BY") {
            return Err(ParseError("ORDER requires BY".into()));
        }
        let key = qualified(&mut parser)?;
        let descending = if parser.keyword("DESC") {
            true
        } else {
            parser.keyword("ASC");
            false
        };
        Some((key, descending))
    } else {
        None
    };
    let limit = if parser.keyword("LIMIT") {
        Some(bound(&mut parser, "LIMIT")?)
    } else {
        None
    };
    let offset = if parser.keyword("OFFSET") {
        if limit.is_none() {
            return Err(ParseError("OFFSET requires LIMIT".into()));
        }
        bound(&mut parser, "OFFSET")?
    } else {
        0
    };
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected JOIN syntax; supported: equi-JOIN, ORDER BY, LIMIT, OFFSET".into(),
        ));
    }
    Ok(Some(JoinSelect {
        left,
        right,
        left_alias,
        right_alias,
        kind,
        columns,
        keys: vec![(first, second)],
        order_by,
        limit,
        offset,
    }))
}

pub fn explain_join(query: &JoinSelect) -> String {
    let conditions = query
        .keys
        .iter()
        .map(|(left, right)| {
            format!(
                "{}.{} = {}.{}",
                left.relation, left.name, right.relation, right.name
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    format!(
        "Source: table {} AS {}\nSource: table {} AS {}\nJoin: {:?} ON {}\nProject: {}\nLimit: {}",
        query.left,
        query.left_alias,
        query.right,
        query.right_alias,
        query.kind,
        conditions,
        query.columns.as_ref().map_or("*".to_owned(), |columns| {
            columns
                .iter()
                .map(|col| format!("{}.{}", col.relation, col.name))
                .collect::<Vec<_>>()
                .join(", ")
        }),
        query
            .limit
            .map_or("unbounded".to_owned(), |n| n.to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inner_joins_with_aliases_and_bounds() {
        let query = parse_join(
            "SELECT u.id, p.label FROM users AS u INNER JOIN plans p ON u.plan_id = p.id ORDER BY u.id DESC LIMIT 5 OFFSET 2;",
        )
        .unwrap().unwrap();
        assert_eq!(query.left, "users");
        assert_eq!(query.right_alias, "p");
        assert_eq!(query.keys[0].0.name, "plan_id");
        assert_eq!(query.limit, Some(5));
        assert_eq!(query.offset, 2);
        assert!(parse_join("SELECT id FROM users").unwrap().is_none());
    }

    #[test]
    fn rejects_ambiguous_or_unsupported_join_syntax() {
        for sql in [
            "SELECT * FROM users JOIN users ON users.id = users.id",
            "SELECT * FROM a JOIN b ON a.id > b.id",
            "SELECT id FROM a JOIN b ON a.id = b.id",
            "SELECT * FROM a JOIN b ON a.id = b.id OR a.id = b.id",
            "SELECT * FROM a JOIN b ON a.id = b.id OFFSET 2",
        ] {
            assert!(parse_join(sql).is_err(), "{sql}");
        }
    }
}
