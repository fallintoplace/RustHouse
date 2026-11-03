//! A bounded correlated AVG predicate over an inner-join row.
use super::{JoinColumn, JoinSelect, ParseError, Parser, Token, alias, projected, tokenize};
use crate::Operator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinCorrelatedAvg {
    pub(crate) outer: JoinSelect,
    pub table: String,
    pub inner_key: String,
    pub inner_value: String,
    pub outer_key: JoinColumn,
    pub outer_value: JoinColumn,
    pub factor: String,
}

impl JoinCorrelatedAvg {
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

/// Parse one conjunctive comparison to an uncorrelated-per-key AVG lookup.
pub fn parse_join_correlated_avg_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<JoinCorrelatedAvg>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut where_index = None;
    let mut end = tokens.len();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
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
    let Some(where_index) = where_index else {
        return Ok(None);
    };
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
                return Ok(None);
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
    let mut found = None;
    let mut remaining = Vec::new();
    for piece in pieces {
        if piece.is_empty() {
            return Err(ParseError("incomplete JOIN WHERE".into()));
        }
        let candidate = piece.iter().any(|token| keyword(token, "SELECT"))
            && piece
                .windows(2)
                .any(|pair| keyword(&pair[0], "AVG") && pair[1] == Token::LParen);
        if !candidate {
            remaining.push(piece);
            continue;
        }
        if found.is_some() {
            return Err(ParseError("JOIN supports one correlated AVG".into()));
        }
        let mut parser = Parser {
            tokens: piece.to_vec(),
            index: 0,
        };
        let outer_value = projected(&mut parser)?;
        if parser.operator()? != Operator::Lt
            || !parser.take(Token::LParen)
            || !parser.keyword("SELECT")
        {
            return Err(ParseError(
                "correlated AVG requires column < (SELECT ...)".into(),
            ));
        }
        let factor = parser.number()?;
        if !factor.parse::<f64>().is_ok_and(f64::is_finite)
            || !parser.take(Token::Star)
            || !parser.keyword("AVG")
            || !parser.take(Token::LParen)
        {
            return Err(ParseError(
                "correlated AVG requires a finite factor and AVG(column)".into(),
            ));
        }
        let inner_value = parser.bounded_word()?;
        if !parser.take(Token::RParen) || !parser.keyword("FROM") {
            return Err(ParseError("correlated AVG requires a table".into()));
        }
        let table = parser.bounded_word()?;
        let inner_alias = alias(&mut parser, &table)?;
        if inner_alias != table || !parser.keyword("WHERE") {
            return Err(ParseError(
                "correlated AVG requires an unaliased table and WHERE".into(),
            ));
        }
        let left = parser.bounded_word()?;
        if parser.operator()? != Operator::Eq {
            return Err(ParseError("correlated AVG requires an equality key".into()));
        }
        let right = parser.bounded_word()?;
        if !parser.take(Token::RParen) || parser.index != parser.tokens.len() {
            return Err(ParseError("unsupported correlated AVG syntax".into()));
        }
        let schema = resolve(&table)?;
        if !schema.contains(&inner_value) {
            return Err(ParseError("unknown correlated AVG column".into()));
        }
        let left_inner = schema.contains(&left);
        let right_inner = schema.contains(&right);
        let (inner_key, outer_key) = match (left_inner, right_inner) {
            (true, false) => (left, right),
            (false, true) => (right, left),
            _ => {
                return Err(ParseError(
                    "correlated AVG requires an unambiguous outer key".into(),
                ));
            }
        };
        found = Some((
            table,
            inner_key,
            inner_value,
            outer_key,
            outer_value,
            factor,
        ));
    }
    let Some((table, inner_key, inner_value, outer_key, outer_value, factor)) = found else {
        return Ok(None);
    };
    let mut outer = tokens[..where_index].to_vec();
    if !remaining.is_empty() {
        outer.push(Token::Word("WHERE".into()));
        for (index, piece) in remaining.iter().enumerate() {
            if index != 0 {
                outer.push(Token::Word("AND".into()));
            }
            outer.extend_from_slice(piece);
        }
    }
    outer.extend_from_slice(&tokens[end..]);
    let rewritten = super::implicit::rewrite(outer, Some(&mut resolve))?;
    let outer = super::parse_join_tokens(rewritten, false)?
        .ok_or_else(|| ParseError("correlated AVG requires an outer JOIN".into()))?;
    if outer.kind != super::JoinType::Inner
        || outer
            .steps
            .iter()
            .any(|step| step.kind != super::JoinType::Inner)
        || outer.scalar_formula.is_none()
        || outer.grouped_count.is_some()
    {
        return Err(ParseError(
            "correlated AVG requires a scalar INNER JOIN aggregate formula".into(),
        ));
    }
    Ok(Some(JoinCorrelatedAvg {
        outer,
        table,
        inner_key,
        inner_value,
        outer_key: JoinColumn {
            relation: String::new(),
            name: outer_key,
        },
        outer_value,
        factor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn avg_column_name_does_not_trigger_avg_function_parser() {
        let sql = "SELECT SUM(a * 1) AS total FROM lefts, rights WHERE left_key = right_key AND a IN (SELECT avg FROM other)";
        assert!(
            parse_join_correlated_avg_with_schema(sql, |_| panic!("schema is unnecessary"))
                .unwrap()
                .is_none()
        );
    }
}
