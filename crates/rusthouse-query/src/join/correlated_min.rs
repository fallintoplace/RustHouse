//! A bounded correlated MIN lookup over joined inputs.
use super::{
    JoinColumn, JoinSelect, JoinType, ParseError, Parser, Token, alias, parse_join_resolved,
    projected, tokenize,
};
use crate::Operator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinCorrelatedMin {
    pub(crate) outer: JoinSelect,
    pub(crate) inner: JoinSelect,
    pub outer_key: JoinColumn,
    pub outer_value: JoinColumn,
    pub inner_key: JoinColumn,
    pub inner_value: JoinColumn,
}
impl JoinCorrelatedMin {
    pub fn outer(&self) -> &JoinSelect {
        &self.outer
    }
    pub fn inner(&self) -> &JoinSelect {
        &self.inner
    }
    pub fn cap_rows(&mut self, cap: usize) {
        self.outer.limit = Some(self.outer.limit.map_or(cap, |limit| limit.min(cap)));
    }
}
fn keyword(token: &Token, name: &str) -> bool {
    matches!(token, Token::Word(word) if word.eq_ignore_ascii_case(name))
}

/// Detect an equality against a SELECT MIN subquery in the top-level WHERE.
pub fn has_join_correlated_min(sql: &str) -> Result<bool, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut in_where = false;
    for (index, token) in tokens.iter().enumerate() {
        if depth == 0 && keyword(token, "WHERE") {
            in_where = true;
        }
        if depth == 0
            && in_where
            && ["GROUP", "ORDER", "LIMIT", "OFFSET"]
                .iter()
                .any(|name| keyword(token, name))
        {
            break;
        }
        if depth == 0
            && in_where
            && *token == Token::Operator(Operator::Eq)
            && tokens.get(index + 1) == Some(&Token::LParen)
            && tokens.get(index + 2).is_some_and(|t| keyword(t, "SELECT"))
            && tokens.get(index + 3).is_some_and(|t| keyword(t, "MIN"))
            && tokens.get(index + 4) == Some(&Token::LParen)
        {
            return Ok(true);
        }
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(false)
}

/// Bind one equality to a grouped per-key MIN from an inner join chain.
pub fn parse_join_correlated_min_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<JoinCorrelatedMin>, ParseError> {
    if !has_join_correlated_min(sql)? {
        return Ok(None);
    }
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut where_index = None;
    let mut end = tokens.len();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| ParseError("unbalanced JOIN WHERE".into()))?
            }
            _ if depth == 0 && where_index.is_none() && keyword(token, "WHERE") => {
                where_index = Some(index)
            }
            Token::Semicolon if depth == 0 && where_index.is_some() => {
                end = index;
                break;
            }
            _ if depth == 0
                && where_index.is_some()
                && ["GROUP", "ORDER", "LIMIT", "OFFSET"]
                    .iter()
                    .any(|name| keyword(token, name)) =>
            {
                end = index;
                break;
            }
            _ => {}
        }
    }
    let where_index =
        where_index.ok_or_else(|| ParseError("correlated MIN requires WHERE".into()))?;
    let mut pieces = Vec::new();
    let mut start = where_index + 1;
    depth = 0;
    for index in where_index + 1..end {
        match &tokens[index] {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| ParseError("unbalanced JOIN WHERE".into()))?
            }
            token if depth == 0 && (keyword(token, "OR") || keyword(token, "BETWEEN")) => {
                return Err(ParseError(
                    "correlated MIN requires conjunctive filters".into(),
                ));
            }
            token if depth == 0 && keyword(token, "AND") => {
                pieces.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(ParseError("unbalanced JOIN WHERE".into()));
    }
    pieces.push(&tokens[start..end]);
    let mut remaining = Vec::new();
    let mut found = None;
    for piece in pieces {
        if piece.is_empty() {
            return Err(ParseError("incomplete JOIN WHERE".into()));
        }
        if !piece.iter().any(|token| keyword(token, "SELECT")) {
            remaining.push(piece);
            continue;
        }
        if found.is_some() {
            return Err(ParseError("JOIN supports one correlated MIN".into()));
        }
        let mut parser = Parser {
            tokens: piece.to_vec(),
            index: 0,
        };
        let outer_value = projected(&mut parser)?;
        if parser.operator()? != Operator::Eq
            || !parser.take(Token::LParen)
            || !parser.keyword("SELECT")
            || !parser.keyword("MIN")
            || !parser.take(Token::LParen)
        {
            return Err(ParseError(
                "correlated MIN requires column = (SELECT MIN(column) ...)".into(),
            ));
        }
        let inner_value = projected(&mut parser)?;
        if inner_value.name == "*" || !parser.take(Token::RParen) || !parser.keyword("FROM") {
            return Err(ParseError(
                "correlated MIN requires MIN(column) and FROM".into(),
            ));
        }
        let from_start = parser.index;
        let mut tables = Vec::new();
        loop {
            let table = parser.bounded_word()?;
            let name = alias(&mut parser, &table)?;
            if tables
                .iter()
                .any(|(_, existing, _): &(String, String, Vec<String>)| existing == &name)
            {
                return Err(ParseError(
                    "correlated MIN requires distinct aliases".into(),
                ));
            }
            let schema = resolve(&table)?;
            tables.push((table, name, schema));
            if tables.len() > 8 {
                return Err(ParseError(
                    "correlated MIN supports at most 8 inputs".into(),
                ));
            }
            if !parser.take(Token::Comma) {
                break;
            }
        }
        let from_end = parser.index;
        if tables.len() < 2 || !parser.keyword("WHERE") {
            return Err(ParseError(
                "correlated MIN requires joined inputs and WHERE".into(),
            ));
        }
        let lookup = |column: &JoinColumn| -> Result<bool, ParseError> {
            if !column.relation.is_empty() {
                if let Some((_, _, schema)) = tables
                    .iter()
                    .find(|(_, alias, _)| alias == &column.relation)
                {
                    if !schema.contains(&column.name) {
                        return Err(ParseError(format!(
                            "unknown inner correlated MIN column: {}.{}",
                            column.relation, column.name
                        )));
                    }
                    return Ok(true);
                }
                return Ok(false);
            }
            let matches = tables
                .iter()
                .filter(|(_, name, schema)| {
                    (column.relation.is_empty() || column.relation == *name)
                        && schema.contains(&column.name)
                })
                .count();
            if matches > 1 {
                return Err(ParseError(format!(
                    "ambiguous correlated MIN column: {}",
                    column.name
                )));
            }
            Ok(matches == 1)
        };
        if !lookup(&inner_value)? {
            return Err(ParseError("unknown correlated MIN value".into()));
        }
        let mut inner_pieces = Vec::new();
        let mut part_start = parser.index;
        let mut level = 0usize;
        let mut closing = None;
        for index in parser.index..parser.tokens.len() {
            match &parser.tokens[index] {
                Token::LParen => level += 1,
                Token::RParen if level == 0 => {
                    inner_pieces.push(&parser.tokens[part_start..index]);
                    closing = Some(index);
                    break;
                }
                Token::RParen => level -= 1,
                token if level == 0 && (keyword(token, "OR") || keyword(token, "BETWEEN")) => {
                    return Err(ParseError(
                        "correlated MIN requires conjunctive inner filters".into(),
                    ));
                }
                token if level == 0 && keyword(token, "AND") => {
                    inner_pieces.push(&parser.tokens[part_start..index]);
                    part_start = index + 1;
                }
                _ => {}
            }
        }
        if closing != Some(parser.tokens.len() - 1) {
            return Err(ParseError("unsupported correlated MIN subquery".into()));
        }
        let mut correlation = None;
        let mut inner_filters = Vec::new();
        for part in inner_pieces {
            if part.is_empty() {
                return Err(ParseError("incomplete correlated MIN filter".into()));
            }
            let mut test = Parser {
                tokens: part.to_vec(),
                index: 0,
            };
            let comparison = projected(&mut test).ok().and_then(|left| {
                if !test.take(Token::Operator(Operator::Eq)) {
                    return None;
                }
                let right = projected(&mut test).ok()?;
                (test.index == test.tokens.len()).then_some((left, right))
            });
            if let Some((left, right)) = comparison {
                let left_inner = lookup(&left)?;
                let right_inner = lookup(&right)?;
                if left_inner != right_inner {
                    if correlation.is_some() {
                        return Err(ParseError(
                            "correlated MIN supports one equality key".into(),
                        ));
                    }
                    correlation = Some(if left_inner {
                        (left, right)
                    } else {
                        (right, left)
                    });
                    continue;
                }
            }
            inner_filters.push(part);
        }
        let (inner_key, outer_key) = correlation
            .ok_or_else(|| ParseError("correlated MIN requires an equality key".into()))?;
        if inner_filters.is_empty() {
            return Err(ParseError("correlated MIN requires inner join keys".into()));
        }
        let w = |name: &str| Token::Word(name.to_owned());
        let column_tokens = |column: &JoinColumn| {
            let mut result = Vec::new();
            if !column.relation.is_empty() {
                result.extend([w(&column.relation), Token::Dot]);
            }
            result.push(w(&column.name));
            result
        };
        let mut inner = vec![w("SELECT")];
        inner.extend(column_tokens(&inner_key));
        inner.extend([Token::Comma, w("MIN"), Token::LParen]);
        inner.extend(column_tokens(&inner_value));
        inner.extend([Token::RParen, w("AS"), w("__correlated_min"), w("FROM")]);
        inner.extend_from_slice(&parser.tokens[from_start..from_end]);
        inner.push(w("WHERE"));
        for (index, filter) in inner_filters.iter().enumerate() {
            if index != 0 {
                inner.push(w("AND"));
            }
            inner.extend_from_slice(filter);
        }
        inner.extend([w("GROUP"), w("BY")]);
        inner.extend(column_tokens(&inner_key));
        let inner = parse_join_resolved(inner, Some(&mut resolve))?
            .ok_or_else(|| ParseError("correlated MIN requires an inner JOIN".into()))?;
        found = Some((outer_value, inner, inner_key, outer_key, inner_value));
    }
    let Some((outer_value, inner, inner_key, outer_key, inner_value)) = found else {
        return Ok(None);
    };
    let mut outer = tokens[..where_index].to_vec();
    if !remaining.is_empty() {
        outer.push(Token::Word("WHERE".into()));
        for (index, filter) in remaining.iter().enumerate() {
            if index != 0 {
                outer.push(Token::Word("AND".into()));
            }
            outer.extend_from_slice(filter);
        }
    }
    outer.extend_from_slice(&tokens[end..]);
    let outer = parse_join_resolved(outer, Some(&mut resolve))?
        .ok_or_else(|| ParseError("correlated MIN requires an outer JOIN".into()))?;
    if outer.kind != JoinType::Inner
        || outer.steps.iter().any(|step| step.kind != JoinType::Inner)
        || outer.aggregate.is_some()
        || outer.grouped_count.is_some()
        || outer.scalar_sum.is_some()
        || outer.scalar_formula.is_some()
        || outer.in_subquery.is_some()
    {
        return Err(ParseError(
            "correlated MIN requires an INNER JOIN projection".into(),
        ));
    }
    Ok(Some(JoinCorrelatedMin {
        outer,
        inner,
        outer_key,
        outer_value,
        inner_key,
        inner_value,
    }))
}
