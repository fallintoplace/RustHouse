//! Convert a bounded conjunctive comma join into the existing inner join plan.
use super::{ParseError, Parser, Token, alias};
use crate::Operator;

type SchemaResolver<'a> = &'a mut (dyn FnMut(&str) -> Result<Vec<String>, ParseError> + 'a);

fn keyword(token: &Token, name: &str) -> bool {
    matches!(token, Token::Word(w) if w.eq_ignore_ascii_case(name))
}

fn clause_end(token: &Token) -> bool {
    ["GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "FORMAT"]
        .iter()
        .any(|name| keyword(token, name))
        || *token == Token::Semicolon
}

fn qualifier(token: &Token) -> Option<&str> {
    match token {
        Token::Word(s) | Token::QuotedWord(s) => Some(s),
        _ => None,
    }
}

fn unqualified_column(token: &Token) -> Option<&str> {
    match token {
        Token::Word(name)
            if [
                "CAST",
                "DATE",
                "TIMESTAMP",
                "NULL",
                "TRUE",
                "FALSE",
                "INTERVAL",
            ]
            .iter()
            .any(|keyword| name.eq_ignore_ascii_case(keyword)) =>
        {
            None
        }
        _ => qualifier(token),
    }
}

fn from_bounds(tokens: &[Token]) -> Result<Option<(usize, usize)>, ParseError> {
    if !matches!(tokens.first(), Some(t) if keyword(t, "SELECT")) {
        return Ok(None);
    }
    let mut depth = 0usize;
    let mut from = None;
    let mut end = None;
    let mut has_comma = false;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && from.is_none() && keyword(token, "FROM") => from = Some(i),
            Token::Comma if depth == 0 && from.is_some() && end.is_none() => has_comma = true,
            _ if depth == 0
                && from.is_some()
                && end.is_none()
                && (keyword(token, "WHERE") || clause_end(token)) =>
            {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    if !has_comma {
        return Ok(None);
    }
    Ok(Some((
        from.expect("comma follows FROM"),
        end.unwrap_or(tokens.len()),
    )))
}

pub(super) fn has_comma(tokens: &[Token]) -> Result<bool, ParseError> {
    Ok(from_bounds(tokens)?.is_some())
}

/// Rewrite AND-connected comma joins into existing inner join plans.
pub(super) fn rewrite(
    tokens: Vec<Token>,
    mut resolve: Option<SchemaResolver<'_>>,
) -> Result<Vec<Token>, ParseError> {
    let Some((from, end)) = from_bounds(&tokens)? else {
        return Ok(tokens);
    };
    if end == tokens.len() || !keyword(&tokens[end], "WHERE") {
        return Err(ParseError("comma joins require WHERE equality keys".into()));
    }
    let mut parser = Parser {
        tokens: tokens[from + 1..end].to_vec(),
        index: 0,
    };
    let mut tables = Vec::<(String, String)>::new();
    loop {
        let name = parser.bounded_word()?;
        let alias = alias(&mut parser, &name)?;
        if tables.iter().any(|(_, existing)| existing == &alias) {
            return Err(ParseError("joined tables require distinct aliases".into()));
        }
        tables.push((name, alias));
        if tables.len() > 8 {
            return Err(ParseError("JOIN supports at most 8 tables".into()));
        }
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if parser.index != parser.tokens.len() || tables.len() < 2 {
        return Err(ParseError("comma join supports named tables only".into()));
    }
    let mut pred_end = end + 1;
    let mut depth = 0usize;
    while pred_end < tokens.len() {
        match &tokens[pred_end] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            t if depth == 0 && clause_end(t) => break,
            _ => {}
        }
        pred_end += 1;
    }
    let predicates = &tokens[end + 1..pred_end];
    if predicates.iter().any(|t| keyword(t, "OR")) {
        return Err(ParseError("comma join with OR is not yet supported".into()));
    }
    let mut pieces = Vec::<Vec<Token>>::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, token) in predicates.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && keyword(token, "AND") => {
                pieces.push(predicates[start..i].to_vec());
                start = i + 1;
            }
            _ => {}
        }
    }
    pieces.push(predicates[start..].to_vec());
    if pieces.iter().any(Vec::is_empty) {
        return Err(ParseError("comma join requires complete predicates".into()));
    }
    let needs_schema = pieces.iter().any(|piece| match piece.as_slice() {
        [left, Token::Operator(Operator::Eq), right] => {
            unqualified_column(left).is_some() && unqualified_column(right).is_some()
        }
        _ => false,
    });
    let schemas = if let (true, Some(resolver)) = (needs_schema, resolve.as_mut()) {
        Some(
            tables
                .iter()
                .map(|(table, _)| resolver(table))
                .collect::<Result<Vec<_>, _>>()?,
        )
    } else {
        None
    };
    let mut keys = vec![Vec::<Vec<Token>>::new(); tables.len()];
    let mut remainder = Vec::new();
    for mut piece in pieces {
        if let (Some(schemas), [left, Token::Operator(Operator::Eq), right]) =
            (&schemas, piece.as_slice())
        {
            if let (Some(left_name), Some(right_name)) =
                (unqualified_column(left), unqualified_column(right))
            {
                let lookup = |name: &str| -> Result<Option<usize>, ParseError> {
                    let found: Vec<_> = schemas
                        .iter()
                        .enumerate()
                        .filter(|(_, cols)| cols.iter().any(|c| c == name))
                        .map(|(i, _)| i)
                        .collect();
                    if found.len() > 1 {
                        return Err(ParseError(format!("ambiguous JOIN column: {name}")));
                    }
                    Ok(found.first().copied())
                };
                if let (Some(a), Some(b)) = (lookup(left_name)?, lookup(right_name)?) {
                    if a != b {
                        piece = vec![
                            Token::Word(tables[a].1.clone()),
                            Token::Dot,
                            left.clone(),
                            Token::Operator(Operator::Eq),
                            Token::Word(tables[b].1.clone()),
                            Token::Dot,
                            right.clone(),
                        ];
                    }
                }
            }
        }
        if let [
            a,
            Token::Dot,
            left,
            Token::Operator(Operator::Eq),
            b,
            Token::Dot,
            right,
        ] = piece.as_slice()
        {
            if qualifier(left).is_some() && qualifier(right).is_some() {
                let ai = qualifier(a)
                    .and_then(|name| tables.iter().position(|(_, alias)| alias == name));
                let bi = qualifier(b)
                    .and_then(|name| tables.iter().position(|(_, alias)| alias == name));
                if let (Some(ai), Some(bi)) = (ai, bi) {
                    if ai != bi {
                        keys[ai.max(bi)].push(piece);
                        continue;
                    }
                }
            }
        }
        remainder.push(piece);
    }
    if keys.iter().skip(1).any(Vec::is_empty) {
        return Err(ParseError(
            "each comma joined table needs a qualified equality key".into(),
        ));
    }
    let word = |s: &str| Token::Word(s.to_owned());
    let mut result = tokens[..from + 1].to_vec();
    result.push(word(&tables[0].0));
    result.push(word("AS"));
    result.push(word(&tables[0].1));
    for (index, (table, name)) in tables.iter().enumerate().skip(1) {
        result.push(word("JOIN"));
        result.push(word(table));
        result.push(word("AS"));
        result.push(word(name));
        result.push(word("ON"));
        for (i, key) in keys[index].iter().enumerate() {
            if i != 0 {
                result.push(word("AND"));
            }
            result.extend(key.iter().cloned());
        }
    }
    if !remainder.is_empty() {
        result.push(word("WHERE"));
        for (i, part) in remainder.iter().enumerate() {
            if i != 0 {
                result.push(word("AND"));
            }
            result.extend(part.iter().cloned());
        }
    }
    result.extend_from_slice(&tokens[pred_end..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::parse_join;
    #[test]
    fn qualified_join_does_not_require_schema_resolution() {
        let parsed = super::super::parse_join_with_schema(
            "SELECT a.id FROM first a, second b WHERE a.id = b.id",
            |_| panic!("qualified join does not need a schema"),
        )
        .unwrap();
        assert!(parsed.is_some());
    }

    #[test]
    fn boolean_literals_are_not_resolved_as_columns() {
        let resolve = |table: &str| {
            Ok(if table == "first" {
                vec!["flag".into()]
            } else {
                vec!["TRUE".into()]
            })
        };
        assert!(
            super::super::parse_join_with_schema(
                "SELECT count(*) FROM first, second WHERE flag = TRUE",
                resolve
            )
            .is_err()
        );
    }

    #[test]
    fn handles_qualified_implicit_joins_without_moving_or_predicates() {
        let parsed = parse_join("SELECT a.id, b.id FROM first a, second b WHERE a.id = b.id")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.keys.len(), 1);
        assert!(
            parse_join("SELECT a.id FROM first a, second b WHERE a.id = b.id OR a.id = 1").is_err()
        );
        assert!(parse_join("SELECT a.id FROM first a, second b WHERE a.id = 1").is_err());
        let chained = parse_join(
            "SELECT a.id FROM first a, second b, third c WHERE b.id = c.id AND a.id = b.id",
        )
        .unwrap()
        .unwrap();
        assert_eq!(chained.steps.len(), 1);
    }
}
