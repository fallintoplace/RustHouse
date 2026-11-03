//! A bounded two-key correlated SUM predicate over joined rows.
use super::{
    JoinColumn, JoinSelect, JoinType, ParseError, Parser, Token, alias, parse_join_resolved,
    projected, tokenize,
};
use crate::{Operator, Predicate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinCorrelatedSum {
    pub(crate) outer: JoinSelect,
    pub table: String,
    pub inner_alias: String,
    pub inner_keys: [JoinColumn; 2],
    pub outer_keys: [JoinColumn; 2],
    pub inner_value: JoinColumn,
    pub outer_value: JoinColumn,
    pub factor: String,
    pub inner_filters: Vec<Vec<Predicate>>,
}
impl JoinCorrelatedSum {
    pub fn outer(&self) -> &JoinSelect {
        &self.outer
    }
    pub fn cap_rows(&mut self, cap: usize) {
        self.outer.limit = Some(self.outer.limit.map_or(cap, |limit| limit.min(cap)));
    }
}
fn keyword(token: &Token, name: &str) -> bool {
    matches!(token, Token::Word(word) if word.eq_ignore_ascii_case(name))
}

/// Detect a top-level comparison to a factored SUM subquery.
pub fn has_join_correlated_sum(sql: &str) -> Result<bool, ParseError> {
    Ok(has_tokens(&tokenize(sql)?))
}

fn has_tokens(tokens: &[Token]) -> bool {
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
            && *token == Token::Operator(Operator::Gt)
            && tokens.get(index + 1) == Some(&Token::LParen)
            && tokens.get(index + 2).is_some_and(|t| keyword(t, "SELECT"))
            && matches!(tokens.get(index + 3), Some(Token::Number(_)))
            && tokens.get(index + 4) == Some(&Token::Star)
            && tokens.get(index + 5).is_some_and(|t| keyword(t, "SUM"))
            && tokens.get(index + 6) == Some(&Token::LParen)
        {
            return true;
        }
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// Parse one comparison against a filtered two-key SUM lookup.
pub fn parse_join_correlated_sum_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<JoinCorrelatedSum>, ParseError> {
    parse_tokens(tokenize(sql)?, &mut resolve)
}

pub(super) fn parse_tokens(
    tokens: Vec<Token>,
    resolve: &mut dyn FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<JoinCorrelatedSum>, ParseError> {
    if !has_tokens(&tokens) {
        return Ok(None);
    }
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
        where_index.ok_or_else(|| ParseError("correlated SUM requires WHERE".into()))?;
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
                    "correlated SUM requires conjunctive filters".into(),
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
            return Err(ParseError("JOIN supports one correlated SUM".into()));
        }
        let mut parser = Parser {
            tokens: piece.to_vec(),
            index: 0,
        };
        let outer_value = projected(&mut parser)?;
        if parser.operator()? != Operator::Gt
            || !parser.take(Token::LParen)
            || !parser.keyword("SELECT")
        {
            return Err(ParseError(
                "correlated SUM requires column > (SELECT ...)".into(),
            ));
        }
        let factor = parser.number()?;
        if !factor.parse::<f64>().is_ok_and(f64::is_finite)
            || !parser.take(Token::Star)
            || !parser.keyword("SUM")
            || !parser.take(Token::LParen)
        {
            return Err(ParseError(
                "correlated SUM requires a finite factor and SUM(column)".into(),
            ));
        }
        let inner_value = projected(&mut parser)?;
        if inner_value.name == "*" || !parser.take(Token::RParen) || !parser.keyword("FROM") {
            return Err(ParseError(
                "correlated SUM requires a value and table".into(),
            ));
        }
        let table = parser.bounded_word()?;
        let inner_alias = alias(&mut parser, &table)?;
        if !parser.keyword("WHERE") {
            return Err(ParseError("correlated SUM requires WHERE".into()));
        }
        let schema = resolve(&table)?;
        let lookup = |column: &JoinColumn| -> Result<bool, ParseError> {
            if !column.relation.is_empty() {
                if column.relation != inner_alias {
                    return Ok(false);
                }
                if !schema.contains(&column.name) {
                    return Err(ParseError(format!(
                        "unknown inner correlated SUM column: {}.{}",
                        column.relation, column.name
                    )));
                }
                return Ok(true);
            }
            Ok(schema.contains(&column.name))
        };
        if !lookup(&inner_value)? {
            return Err(ParseError("unknown correlated SUM value".into()));
        }
        let mut inner_parts = Vec::new();
        let mut part_start = parser.index;
        let mut level = 0usize;
        let mut closing = None;
        for index in parser.index..parser.tokens.len() {
            match &parser.tokens[index] {
                Token::LParen => level += 1,
                Token::RParen if level == 0 => {
                    inner_parts.push(&parser.tokens[part_start..index]);
                    closing = Some(index);
                    break;
                }
                Token::RParen => level -= 1,
                token if level == 0 && (keyword(token, "OR") || keyword(token, "BETWEEN")) => {
                    return Err(ParseError(
                        "correlated SUM requires conjunctive inner filters".into(),
                    ));
                }
                token if level == 0 && keyword(token, "AND") => {
                    inner_parts.push(&parser.tokens[part_start..index]);
                    part_start = index + 1;
                }
                _ => {}
            }
        }
        if closing != Some(parser.tokens.len() - 1) {
            return Err(ParseError("unsupported correlated SUM subquery".into()));
        }
        let mut correlations = Vec::new();
        let mut filters = Vec::new();
        for part in inner_parts {
            if part.is_empty() {
                return Err(ParseError("incomplete correlated SUM filter".into()));
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
                let a = lookup(&left)?;
                let b = lookup(&right)?;
                if a != b {
                    correlations.push(if a { (left, right) } else { (right, left) });
                    continue;
                }
            }
            filters.push(part);
        }
        if correlations.len() != 2
            || correlations[0].0 == correlations[1].0
            || correlations[0].1 == correlations[1].1
        {
            return Err(ParseError(
                "correlated SUM requires two distinct equality keys".into(),
            ));
        }
        let mut filter_tokens = Vec::new();
        for (index, part) in filters.iter().enumerate() {
            if index != 0 {
                filter_tokens.push(Token::Word("AND".into()));
            }
            filter_tokens.extend_from_slice(part);
        }
        let inner_filters = if filter_tokens.is_empty() {
            vec![vec![]]
        } else {
            let mut filter_parser = Parser {
                tokens: filter_tokens,
                index: 0,
            };
            let branches = crate::where_parser::parse_join_where(&mut filter_parser)?;
            if filter_parser.index != filter_parser.tokens.len() {
                return Err(ParseError("invalid correlated SUM filter".into()));
            }
            branches
        };
        found = Some((
            outer_value,
            table,
            inner_alias,
            inner_value,
            factor,
            correlations,
            inner_filters,
        ));
    }
    let Some((outer_value, table, inner_alias, inner_value, factor, correlations, inner_filters)) =
        found
    else {
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
    let outer = parse_join_resolved(outer, Some(resolve))?
        .ok_or_else(|| ParseError("correlated SUM requires an outer JOIN".into()))?;
    if outer.kind != JoinType::Inner
        || outer.steps.iter().any(|step| step.kind != JoinType::Inner)
        || outer.aggregate.is_some()
        || outer.grouped_count.is_some()
        || outer.scalar_sum.is_some()
        || outer.scalar_formula.is_some()
        || outer.in_subquery.is_some()
    {
        return Err(ParseError(
            "correlated SUM requires an INNER JOIN projection".into(),
        ));
    }
    Ok(Some(JoinCorrelatedSum {
        outer,
        table,
        inner_alias,
        inner_keys: [correlations[0].0.clone(), correlations[1].0.clone()],
        outer_keys: [correlations[0].1.clone(), correlations[1].1.clone()],
        inner_value,
        outer_value,
        factor,
        inner_filters,
    }))
}
