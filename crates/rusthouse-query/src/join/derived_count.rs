//! Parse a grouped COUNT over the results of a grouped LEFT JOIN.
use super::{
    JoinColumn, JoinSelect, JoinType, ParseError, Parser, Token, alias, parse_join_kind, projected,
    tokenize,
};
use crate::{AggregateFunction, Projection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedJoinCount {
    pub(crate) inner: JoinSelect,
    pub(crate) group_name: String,
    pub(crate) count_name: String,
    /// True orders by the outer count; false orders by the inner count.
    pub(crate) order: Vec<(bool, bool)>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: usize,
}

impl DerivedJoinCount {
    pub fn inner(&self) -> &JoinSelect {
        &self.inner
    }
    pub fn cap_rows(&mut self, cap: usize) {
        self.limit = Some(self.limit.map_or(cap, |limit| limit.min(cap)));
    }
}

fn keyword(token: &Token, name: &str) -> bool {
    matches!(token, Token::Word(word) if word.eq_ignore_ascii_case(name))
}

fn derived_from(tokens: &[Token]) -> Option<usize> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && keyword(token, "FROM") => {
                return (tokens.get(index + 1) == Some(&Token::LParen)
                    && tokens
                        .get(index + 2)
                        .is_some_and(|token| keyword(token, "SELECT")))
                .then_some(index);
            }
            _ => {}
        }
    }
    None
}

/// Whether the outer SELECT uses a derived SELECT source.
pub fn has_derived_join_count(sql: &str) -> Result<bool, ParseError> {
    Ok(derived_from(&tokenize(sql)?).is_some())
}

/// Parse a grouped COUNT over the count produced by a grouped two-table LEFT JOIN.
pub fn parse_derived_join_count_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<DerivedJoinCount>, ParseError> {
    let tokens = tokenize(sql)?;
    let Some(from) = derived_from(&tokens) else {
        return Ok(None);
    };
    let mut end = from + 2;
    let mut depth = 1usize;
    while end < tokens.len() && depth > 0 {
        match tokens[end] {
            Token::LParen => depth += 1,
            Token::RParen => depth -= 1,
            _ => {}
        }
        end += 1;
    }
    if depth != 0 {
        return Err(ParseError("derived JOIN requires closing ')'".into()));
    }
    let mut inner_tokens = tokens[from + 2..end - 1].to_vec();
    let mut level = 0usize;
    let inner_from = inner_tokens
        .iter()
        .enumerate()
        .find_map(|(index, token)| {
            match token {
                Token::LParen => level += 1,
                Token::RParen => level = level.saturating_sub(1),
                _ => {}
            }
            (level == 0 && keyword(token, "FROM")).then_some(index)
        })
        .ok_or_else(|| ParseError("derived JOIN requires FROM".into()))?;
    let mut parser = Parser {
        tokens: inner_tokens.clone(),
        index: inner_from + 1,
    };
    let left = parser.bounded_word()?;
    let left_alias = alias(&mut parser, &left)?;
    if parse_join_kind(&mut parser) != JoinType::Left || !parser.keyword("JOIN") {
        return Err(ParseError("derived count requires LEFT JOIN".into()));
    }
    let right = parser.bounded_word()?;
    let right_alias = alias(&mut parser, &right)?;
    if left_alias == right_alias || !parser.keyword("ON") {
        return Err(ParseError(
            "derived JOIN requires distinct aliases and ON".into(),
        ));
    }
    let key_start = parser.index;
    let first = projected(&mut parser)?;
    if !parser.take(Token::Operator(crate::Operator::Eq)) {
        return Err(ParseError("derived JOIN requires an equality key".into()));
    }
    let second = projected(&mut parser)?;
    let key_end = parser.index;
    let left_schema = resolve(&left)?;
    let right_schema = resolve(&right)?;
    let qualify = |column: JoinColumn| -> Result<JoinColumn, ParseError> {
        let in_left = left_schema.contains(&column.name);
        let in_right = right_schema.contains(&column.name);
        let relation = if column.relation.is_empty() {
            match (in_left, in_right) {
                (true, false) => left_alias.clone(),
                (false, true) => right_alias.clone(),
                (true, true) => {
                    return Err(ParseError(format!(
                        "ambiguous JOIN column: {}",
                        column.name
                    )));
                }
                _ => return Err(ParseError(format!("unknown JOIN column: {}", column.name))),
            }
        } else if (column.relation == left_alias && in_left)
            || (column.relation == right_alias && in_right)
        {
            column.relation
        } else {
            return Err(ParseError(
                "unknown derived JOIN qualifier or column".into(),
            ));
        };
        Ok(JoinColumn {
            relation,
            name: column.name,
        })
    };
    let first = qualify(first)?;
    let second = qualify(second)?;
    if first.relation == second.relation {
        return Err(ParseError(
            "derived JOIN requires one key from each input".into(),
        ));
    }
    inner_tokens.splice(
        key_start..key_end,
        [
            Token::Word(first.relation),
            Token::Dot,
            Token::Word(first.name),
            Token::Operator(crate::Operator::Eq),
            Token::Word(second.relation),
            Token::Dot,
            Token::Word(second.name),
        ],
    );
    let inner = super::parse_join_tokens(inner_tokens, false)?
        .ok_or_else(|| ParseError("derived count requires a JOIN".into()))?;
    if inner.kind != JoinType::Left
        || !inner.steps.is_empty()
        || inner
            .grouped_count
            .as_ref()
            .is_none_or(|keys| keys.len() != 1)
        || inner.grouped_count_column.is_none()
        || inner.limit.is_some()
        || inner.offset != 0
        || !inner.order_by.is_empty()
        || inner.where_null.is_some()
        || inner.where_comparison.is_some()
        || inner.scalar_where.is_some()
    {
        return Err(ParseError("unsupported derived grouped JOIN".into()));
    }
    let mut suffix = Parser {
        tokens: tokens[end..].to_vec(),
        index: 0,
    };
    suffix.keyword("AS");
    let _relation = suffix.bounded_word()?;
    if !suffix.take(Token::LParen) {
        return Err(ParseError(
            "derived JOIN requires two column aliases".into(),
        ));
    }
    let first_alias = suffix.bounded_word()?;
    if !suffix.take(Token::Comma) {
        return Err(ParseError(
            "derived JOIN requires two column aliases".into(),
        ));
    }
    let group_name = suffix.bounded_word()?;
    if first_alias.eq_ignore_ascii_case(&group_name) || !suffix.take(Token::RParen) {
        return Err(ParseError("invalid derived column aliases".into()));
    }
    let mut outer_tokens = tokens[..from + 1].to_vec();
    outer_tokens.push(Token::Word("part".into()));
    outer_tokens.extend_from_slice(&suffix.tokens[suffix.index..]);
    let outer = crate::parse_select_tokens(outer_tokens)?;
    let count_name = match &outer.projection {
        Projection::GroupedCount { column } if column == &group_name => {
            outer.alias.clone().unwrap_or_else(|| "count".into())
        }
        Projection::GroupedAggregates {
            column,
            items,
            aliases,
            filters,
        } if column == &group_name
            && items.as_slice() == [(AggregateFunction::Count, "*".into())]
            && filters == &[None]
            && aliases.len() == 1 =>
        {
            aliases[0].clone().unwrap_or_else(|| "count".into())
        }
        _ => {
            return Err(ParseError(
                "derived JOIN requires GROUP BY inner count with COUNT(*)".into(),
            ));
        }
    };
    if group_name.eq_ignore_ascii_case("count")
        || group_name.eq_ignore_ascii_case("#aggregate0")
        || group_name.eq_ignore_ascii_case(&count_name)
        || group_name.chars().any(char::is_control)
        || count_name.chars().any(char::is_control)
    {
        return Err(ParseError(
            "invalid or ambiguous derived output names".into(),
        ));
    }
    if outer.distinct
        || !outer.predicates.is_empty()
        || !outer.or_predicates.is_empty()
        || outer.having.is_some()
        || outer.hidden_aggregate
        || outer.rename.is_some()
        || !outer.grouped_expressions.is_empty()
    {
        return Err(ParseError(
            "unsupported derived grouped count filter".into(),
        ));
    }
    let mut order = Vec::new();
    for term in outer.order_terms() {
        let is_count =
            term.column == "count" || term.column == "#aggregate0" || term.column == count_name;
        if !is_count && term.column != group_name {
            return Err(ParseError(
                "derived count ORDER BY requires an output column".into(),
            ));
        }
        order.push((is_count, term.descending));
    }
    Ok(Some(DerivedJoinCount {
        inner,
        group_name,
        count_name,
        order,
        limit: outer.limit,
        offset: outer.offset.unwrap_or(0),
    }))
}
