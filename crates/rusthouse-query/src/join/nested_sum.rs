//! Flatten a nested IN over a two-key correlated SUM.
use super::{
    JoinColumn, JoinCorrelatedSum, JoinSelect, JoinType, ParseError, Parser, Token, alias, bound,
    parse_join_resolved, projected, tokenize,
};
use crate::Operator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedJoinSum {
    pub(crate) outer: JoinSelect,
    pub(crate) inner: JoinCorrelatedSum,
    pub outer_key: JoinColumn,
    pub inner_key: JoinColumn,
}
impl NestedJoinSum {
    pub fn outer(&self) -> &JoinSelect {
        &self.outer
    }
    pub fn inner(&self) -> &JoinCorrelatedSum {
        &self.inner
    }
    pub fn cap_rows(&mut self, cap: usize) {
        self.outer.limit = Some(self.outer.limit.map_or(cap, |limit| limit.min(cap)));
    }
}
fn kw(token: &Token, name: &str) -> bool {
    matches!(token, Token::Word(word) if word.eq_ignore_ascii_case(name))
}
fn word(name: &str) -> Token {
    Token::Word(name.to_owned())
}
fn column_tokens(column: &JoinColumn) -> Vec<Token> {
    let mut result = Vec::new();
    if !column.relation.is_empty() {
        result.extend([word(&column.relation), Token::Dot]);
    }
    result.push(word(&column.name));
    result
}
fn parts(tokens: &[Token]) -> Result<Vec<&[Token]>, ParseError> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| ParseError("unbalanced nested IN".into()))?
            }
            _ if depth == 0 && (kw(token, "OR") || kw(token, "BETWEEN")) => {
                return Err(ParseError("nested IN requires conjunctive filters".into()));
            }
            _ if depth == 0 && kw(token, "AND") => {
                result.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(ParseError("unbalanced nested IN".into()));
    }
    result.push(&tokens[start..]);
    if result.iter().any(|part| part.is_empty()) {
        return Err(ParseError("incomplete nested IN filter".into()));
    }
    Ok(result)
}
fn is_nested(piece: &[Token]) -> bool {
    piece
        .windows(3)
        .filter(|p| kw(&p[0], "IN") && p[1] == Token::LParen && kw(&p[2], "SELECT"))
        .count()
        == 2
        && piece
            .windows(2)
            .any(|p| kw(&p[0], "SUM") && p[1] == Token::LParen)
}
/// Detect a nested IN containing a SUM subquery.
pub fn has_nested_join_sum(sql: &str) -> Result<bool, ParseError> {
    let tokens = tokenize(sql)?;
    Ok(tokens
        .windows(3)
        .filter(|p| kw(&p[0], "IN") && p[1] == Token::LParen && kw(&p[2], "SELECT"))
        .count()
        >= 2
        && tokens
            .windows(2)
            .any(|p| kw(&p[0], "SUM") && p[1] == Token::LParen))
}
/// Parse one nested IN whose inner filter is a two-key correlated SUM.
pub fn parse_nested_join_sum_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<NestedJoinSum>, ParseError> {
    if !has_nested_join_sum(sql)? {
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
                    .ok_or_else(|| ParseError("unbalanced nested IN".into()))?
            }
            _ if depth == 0 && where_index.is_none() && kw(token, "WHERE") => {
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
                    .any(|name| kw(token, name)) =>
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
    let mut candidates = Vec::new();
    let mut outer_filters = Vec::new();
    for piece in parts(&tokens[where_index + 1..end])? {
        if is_nested(piece) {
            candidates.push(piece);
        } else {
            outer_filters.push(piece);
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    if candidates.len() != 1 || outer_filters.is_empty() {
        return Err(ParseError(
            "nested IN requires one filter and outer join keys".into(),
        ));
    }
    let mut parser = Parser {
        tokens: candidates[0].to_vec(),
        index: 0,
    };
    let outer_key = projected(&mut parser)?;
    if !parser.keyword("IN") || !parser.take(Token::LParen) || !parser.keyword("SELECT") {
        return Err(ParseError("nested IN requires a SELECT".into()));
    }
    let inner_key = projected(&mut parser)?;
    if inner_key.name == "*" || !parser.keyword("FROM") {
        return Err(ParseError(
            "nested IN requires a projected key and table".into(),
        ));
    }
    let stock = parser.bounded_word()?;
    let stock_alias = alias(&mut parser, &stock)?;
    if !parser.keyword("WHERE") {
        return Err(ParseError("nested IN requires inner WHERE".into()));
    }
    let inner_end = parser
        .tokens
        .len()
        .checked_sub(1)
        .ok_or_else(|| ParseError("nested IN requires ')'".into()))?;
    if parser.tokens[inner_end] != Token::RParen {
        return Err(ParseError("nested IN requires ')'".into()));
    }
    let mut membership = None;
    let mut sum = None;
    for piece in parts(&parser.tokens[parser.index..inner_end])? {
        if piece.iter().any(|token| kw(token, "IN")) {
            if membership.replace(piece).is_some() {
                return Err(ParseError("nested IN supports one inner membership".into()));
            }
        } else if piece.iter().any(|token| kw(token, "SELECT")) {
            if sum.replace(piece).is_some() {
                return Err(ParseError("nested IN supports one inner SUM".into()));
            }
        } else {
            return Err(ParseError("unsupported nested IN filter".into()));
        }
    }
    let membership = membership.ok_or_else(|| ParseError("missing inner IN".into()))?;
    let sum = sum.ok_or_else(|| ParseError("missing correlated SUM".into()))?;
    let mut part = Parser {
        tokens: membership.to_vec(),
        index: 0,
    };
    let stock_key = projected(&mut part)?;
    if !part.keyword("IN") || !part.take(Token::LParen) || !part.keyword("SELECT") {
        return Err(ParseError("inner IN requires SELECT".into()));
    }
    let part_key = projected(&mut part)?;
    if !part.keyword("FROM") {
        return Err(ParseError("inner IN requires a table".into()));
    }
    let table = part.bounded_word()?;
    let table_alias = alias(&mut part, &table)?;
    if !part.keyword("WHERE") || table_alias == stock_alias {
        return Err(ParseError(
            "inner IN requires WHERE and distinct aliases".into(),
        ));
    }
    let part_end = part
        .tokens
        .len()
        .checked_sub(1)
        .ok_or_else(|| ParseError("inner IN requires ')'".into()))?;
    if part.tokens[part_end] != Token::RParen || part.index == part_end
        || part.tokens[part.index..part_end].iter().any(|t| matches!(t, Token::Word(w) if ["SELECT", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET"].iter().any(|n| w.eq_ignore_ascii_case(n))))
    { return Err(ParseError("unsupported inner IN query".into())); }
    let stock_schema = resolve(&stock)?;
    let part_schema = resolve(&table)?;
    let qualify =
        |column: &JoinColumn, alias: &str, schema: &[String]| -> Result<JoinColumn, ParseError> {
            if (!column.relation.is_empty() && column.relation != alias)
                || !schema.contains(&column.name)
            {
                return Err(ParseError("unknown nested IN key".into()));
            }
            Ok(JoinColumn {
                relation: alias.to_owned(),
                name: column.name.clone(),
            })
        };
    let stock_key = qualify(&stock_key, &stock_alias, &stock_schema)?;
    let inner_key = qualify(&inner_key, &stock_alias, &stock_schema)?;
    let part_key = qualify(&part_key, &table_alias, &part_schema)?;
    let mut inner = vec![word("SELECT")];
    inner.extend(column_tokens(&inner_key));
    inner.extend([
        word("FROM"),
        word(&stock),
        word("AS"),
        word(&stock_alias),
        word("JOIN"),
        word(&table),
        word("AS"),
        word(&table_alias),
        word("ON"),
    ]);
    inner.extend(column_tokens(&stock_key));
    inner.push(Token::Operator(Operator::Eq));
    inner.extend(column_tokens(&part_key));
    inner.push(word("WHERE"));
    inner.extend_from_slice(&part.tokens[part.index..part_end]);
    inner.push(word("AND"));
    inner.extend_from_slice(sum);
    let inner = super::correlated_sum::parse_tokens(inner, &mut resolve)?
        .ok_or_else(|| ParseError("nested IN requires a correlated SUM".into()))?;
    // The membership table belongs to a sibling subquery and is not visible
    // to the correlated SUM, even though flattening joins it here.
    for column in inner
        .outer_keys
        .iter()
        .chain(std::iter::once(&inner.outer_value))
    {
        qualify(column, &stock_alias, &stock_schema)?;
    }
    let mut outer = tokens[..where_index].to_vec();
    outer.push(word("WHERE"));
    for (index, filter) in outer_filters.iter().enumerate() {
        if index != 0 {
            outer.push(word("AND"));
        }
        outer.extend_from_slice(filter);
    }
    let mut outer = parse_join_resolved(outer, Some(&mut resolve), false)?
        .ok_or_else(|| ParseError("nested IN requires an outer JOIN".into()))?;
    if outer.kind != JoinType::Inner
        || outer.steps.iter().any(|step| step.kind != JoinType::Inner)
        || outer.aggregate.is_some()
        || outer.grouped_count.is_some()
        || outer.scalar_sum.is_some()
        || outer.scalar_formula.is_some()
        || outer.in_subquery.is_some()
    {
        return Err(ParseError(
            "nested IN requires an INNER JOIN projection".into(),
        ));
    }
    let mut suffix = Parser {
        tokens: tokens[end..].to_vec(),
        index: 0,
    };
    if suffix.keyword("ORDER") {
        if !suffix.keyword("BY") {
            return Err(ParseError("ORDER requires BY".into()));
        }
        loop {
            if outer.order_by.len() == 8 {
                return Err(ParseError("ORDER BY supports at most 8 keys".into()));
            }
            let column = projected(&mut suffix)?;
            if column.name == "*" {
                return Err(ParseError("ORDER BY requires a column".into()));
            }
            let descending = if suffix.keyword("DESC") {
                true
            } else {
                suffix.keyword("ASC");
                false
            };
            let nulls = if suffix.keyword("NULLS") {
                if suffix.keyword("FIRST") {
                    Some(true)
                } else if suffix.keyword("LAST") {
                    Some(false)
                } else {
                    return Err(ParseError("invalid NULL ordering".into()));
                }
            } else {
                None
            };
            outer.order_by.push((column, descending, nulls));
            if !suffix.take(Token::Comma) {
                break;
            }
        }
    }
    if suffix.keyword("LIMIT") {
        outer.limit = Some(bound(&mut suffix, "LIMIT")?);
    }
    if suffix.keyword("OFFSET") {
        outer.offset = bound(&mut suffix, "OFFSET")?;
    }
    suffix.take(Token::Semicolon);
    if suffix.index != suffix.tokens.len() {
        return Err(ParseError("unsupported nested IN clause".into()));
    }
    Ok(Some(NestedJoinSum {
        outer,
        inner,
        outer_key,
        inner_key,
    }))
}
