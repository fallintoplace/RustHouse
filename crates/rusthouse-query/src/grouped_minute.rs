//! Parse a grouped count by UTC minute.
use crate::lexer::Token;
use crate::{AggregateFunction, ParseError, Projection, Select, parse_select_tokens};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedMinute {
    pub column: String,
    pub label: String,
}
fn expression(tokens: &[Token], i: usize, column: &str) -> bool {
    matches!(tokens.get(i..i+6), Some([Token::Word(name), Token::LParen, Token::String(unit), Token::Comma, Token::Word(col), Token::RParen]) if name.eq_ignore_ascii_case("DATE_TRUNC") && unit.eq_ignore_ascii_case("minute") && col == column)
}
fn clause_by_at(tokens: &[Token], index: usize, keyword: &str) -> bool {
    matches!(tokens.get(index), Some(Token::Word(word)) if word.eq_ignore_ascii_case(keyword))
        && matches!(tokens.get(index + 1), Some(Token::Word(by)) if by.eq_ignore_ascii_case("BY"))
}

fn top_level_group_order(tokens: &[Token]) -> (Option<usize>, Option<usize>) {
    let mut group = None;
    let mut order = None;
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(_) if depth == 0 && clause_by_at(tokens, i, "GROUP") => {
                group = Some(i + 2)
            }
            Token::Word(_) if depth == 0 && clause_by_at(tokens, i, "ORDER") => {
                order = Some(i + 2)
            }
            _ => {}
        }
    }
    (group, order)
}

fn valid_count_projection(projection: &Projection, column: &str) -> bool {
    match projection {
        Projection::GroupedCount { column: name } => name == column,
        Projection::GroupedAggregates {
            column: name,
            items,
            ..
        } => {
            name == column
                && matches!(items.as_slice(), [(AggregateFunction::Count, value)] if value == "*")
        }
        _ => false,
    }
}

pub(super) fn parse(mut tokens: Vec<Token>) -> Result<Option<Select>, ParseError> {
    let (column, label) = match tokens.as_slice() {
        [
            Token::Word(select),
            Token::Word(name),
            Token::LParen,
            Token::String(unit),
            Token::Comma,
            Token::Word(column),
            Token::RParen,
            Token::Word(as_word),
            Token::Word(alias),
            Token::Comma,
            Token::Word(count),
            ..,
        ] if select.eq_ignore_ascii_case("SELECT")
            && name.eq_ignore_ascii_case("DATE_TRUNC")
            && unit.eq_ignore_ascii_case("minute")
            && as_word.eq_ignore_ascii_case("AS")
            && count.eq_ignore_ascii_case("COUNT") =>
        {
            (column.clone(), alias.clone())
        }
        _ => return Ok(None),
    };
    let (group, order) = top_level_group_order(&tokens);
    let Some(group) = group else { return Ok(None) };
    if !expression(&tokens, group, &column) || matches!(tokens.get(group + 6), Some(Token::Comma)) {
        return Ok(None);
    }
    if let Some(order) = order {
        if !expression(&tokens, order, &column) {
            return Ok(None);
        }
        tokens.splice(order..order + 6, [Token::Word(column.clone())]);
    }
    tokens.splice(group..group + 6, [Token::Word(column.clone())]);
    tokens.splice(1..9, [Token::Word(column.clone())]);
    let mut select = parse_select_tokens(tokens)?;
    let valid = valid_count_projection(&select.projection, &column);
    if !valid || select.having.is_some() || !select.order_by_extra.is_empty() {
        return Err(ParseError(
            "minute grouping supports one COUNT and ordering by the minute".into(),
        ));
    }
    if select
        .order_by
        .as_ref()
        .is_some_and(|order| order.column != column)
    {
        return Err(ParseError(
            "minute grouping orders only by the minute".into(),
        ));
    }
    select.grouped_minute = Some(GroupedMinute { column, label });
    Ok(Some(select))
}
