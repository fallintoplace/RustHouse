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

// A shared equality in every OR branch can be used as a join key without
// changing which rows the original WHERE expression accepts.
fn rewrite_disjunctive(
    tokens: &[Token],
    from: usize,
    end: usize,
    pred_end: usize,
    tables: &[(String, String)],
    resolver: Option<SchemaResolver<'_>>,
) -> Result<Vec<Token>, ParseError> {
    if tables.len() != 2 {
        return Err(ParseError("OR comma joins require two tables".into()));
    }
    let resolver =
        resolver.ok_or_else(|| ParseError("OR comma joins require schema resolution".into()))?;
    let schemas = tables
        .iter()
        .map(|(table, _)| resolver(table))
        .collect::<Result<Vec<_>, _>>()?;
    let mut parser = Parser {
        tokens: tokens[end + 1..pred_end].to_vec(),
        index: 0,
    };
    let branches = crate::where_parser::parse_where(&mut parser)?;
    if parser.index != parser.tokens.len() || branches.is_empty() {
        return Err(ParseError(
            "OR comma join requires complete predicates".into(),
        ));
    }
    let lookup = |column: &str| -> Result<Option<usize>, ParseError> {
        let found = schemas
            .iter()
            .enumerate()
            .filter_map(|(i, cols)| cols.iter().any(|name| name == column).then_some(i))
            .collect::<Vec<_>>();
        if found.len() > 1 {
            return Err(ParseError(format!("ambiguous JOIN column: {column}")));
        }
        Ok(found.first().copied())
    };
    let mut key = None;
    for predicate in &branches[0] {
        let crate::Predicate::ColumnComparison {
            column,
            operator: Operator::Eq,
            other,
        } = predicate
        else {
            continue;
        };
        let (Some(left), Some(right)) = (lookup(column)?, lookup(other)?) else {
            continue;
        };
        if left == right {
            continue;
        }
        let shared = branches.iter().all(|branch| branch.iter().any(|entry| {
            matches!(entry, crate::Predicate::ColumnComparison { column: a, operator: Operator::Eq, other: b }
                if (a == column && b == other) || (a == other && b == column))
        }));
        if shared {
            key = Some(if left == 0 {
                (column.clone(), other.clone())
            } else {
                (other.clone(), column.clone())
            });
            break;
        }
    }
    let (left_key, right_key) =
        key.ok_or_else(|| ParseError("OR comma join requires a shared equality key".into()))?;
    let word = |s: &str| Token::Word(s.to_owned());
    let mut result = tokens[..from + 1].to_vec();
    result.extend([
        word(&tables[0].0),
        word("AS"),
        word(&tables[0].1),
        word("JOIN"),
        word(&tables[1].0),
        word("AS"),
        word(&tables[1].1),
        word("ON"),
        word(&tables[0].1),
        Token::Dot,
        word(&left_key),
        Token::Operator(Operator::Eq),
        word(&tables[1].1),
        Token::Dot,
        word(&right_key),
        word("WHERE"),
    ]);
    result.extend_from_slice(&tokens[end + 1..]);
    Ok(result)
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
    let mut depth = 0usize;
    let mut top_level_or = false;
    for token in predicates {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && keyword(token, "OR") => top_level_or = true,
            _ => {}
        }
    }
    if top_level_or {
        return rewrite_disjunctive(&tokens, from, end, pred_end, &tables, resolve);
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
    let mut edges = Vec::<(usize, usize, Vec<Token>)>::new();
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
                        edges.push((ai, bi, piece));
                        continue;
                    }
                }
            }
        }
        remainder.push(piece);
    }
    // Choose a connected left-deep order. A later table can connect two
    // earlier FROM entries, so the written order is not always executable.
    let mut order = vec![0usize];
    while order.len() < tables.len() {
        let next = (1..tables.len()).find(|candidate| {
            !order.contains(candidate)
                && edges.iter().any(|(a, b, _)| {
                    (*a == *candidate && order.contains(b))
                        || (*b == *candidate && order.contains(a))
                })
        });
        match next {
            Some(index) => order.push(index),
            None => {
                if predicates.iter().any(|token| keyword(token, "OR")) {
                    return rewrite_disjunctive(&tokens, from, end, pred_end, &tables, resolve);
                }
                return Err(ParseError(
                    "comma joins require a connected graph of equality keys".into(),
                ));
            }
        }
    }
    let changed_order = order.iter().enumerate().any(|(i, &index)| i != index);
    // Wildcard output follows the original FROM order. Do not silently change it.
    if changed_order
        && matches!(
            &tokens[1..from],
            [Token::Star] | [Token::Word(_), Token::Star]
        )
    {
        return Err(ParseError(
            "reordered comma joins require explicit projections".into(),
        ));
    }
    let mut position = vec![0usize; tables.len()];
    for (index, &original) in order.iter().enumerate() {
        position[original] = index;
    }
    let mut keys = vec![Vec::<Vec<Token>>::new(); tables.len()];
    for (left, right, piece) in edges {
        keys[position[left].max(position[right])].push(piece);
    }
    let word = |s: &str| Token::Word(s.to_owned());
    let mut result = tokens[..from + 1].to_vec();
    let (first_table, first_alias) = &tables[order[0]];
    result.push(word(first_table));
    result.push(word("AS"));
    result.push(word(first_alias));
    for (index, &original) in order.iter().enumerate().skip(1) {
        let (table, name) = &tables[original];
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
    fn or_comma_join_requires_a_shared_equality_in_every_branch() {
        let schema = |table: &str| {
            Ok(match table {
                "first" => vec!["a_id".into(), "amount".into()],
                "second" => vec!["b_id".into(), "kind".into()],
                _ => vec![],
            })
        };
        let good = "SELECT SUM(amount * 1) FROM first, second WHERE (a_id = b_id AND kind = 'a') OR (b_id = a_id AND kind = 'b')";
        assert!(
            super::super::parse_join_with_schema(good, schema)
                .unwrap()
                .is_some()
        );
        let unsafe_query = "SELECT SUM(amount * 1) FROM first, second WHERE (a_id = b_id AND kind = 'a') OR kind = 'b'";
        assert!(super::super::parse_join_with_schema(unsafe_query, schema).is_err());
        let wrapped = "SELECT SUM(amount * 1) FROM first, second WHERE ((a_id = b_id AND kind = 'a') OR (b_id = a_id AND kind = 'b'))";
        assert!(
            super::super::parse_join_with_schema(wrapped, schema)
                .unwrap()
                .is_some()
        );
    }

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
