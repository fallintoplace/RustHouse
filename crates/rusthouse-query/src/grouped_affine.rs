//! Parse redundant affine keys that depend on the first grouped integer key.
use crate::lexer::Token;
use crate::{ParseError, Projection, Select, parse_select_tokens};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedAffineKey {
    pub label: String,
    pub offset: i64,
}

// Ordering by original positional expressions requires separate binding.
fn has_positional_order(tokens: &[Token], group_end: usize) -> bool {
    let mut depth = 0usize;
    for i in group_end..tokens.len().saturating_sub(2) {
        match &tokens[i] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word)
                if depth == 0
                    && word.eq_ignore_ascii_case("ORDER")
                    && matches!(tokens.get(i+1), Some(Token::Word(by)) if by.eq_ignore_ascii_case("BY")) =>
            {
                return tokens[i + 2..]
                    .iter()
                    .take_while(|token| !matches!(token, Token::Word(word) if ["LIMIT", "OFFSET", "FETCH", "FORMAT"].iter().any(|end| word.eq_ignore_ascii_case(end))) && !matches!(token, Token::Semicolon))
                    .any(|token| matches!(token, Token::Number(_)));
            }
            _ => {}
        }
    }
    false
}

pub(super) fn parse(mut tokens: Vec<Token>) -> Result<Option<Select>, ParseError> {
    let key = match tokens.as_slice() {
        [
            Token::Word(select),
            Token::Word(key),
            Token::Comma,
            Token::Word(again),
            Token::Minus | Token::Plus,
            Token::Number(_),
            ..,
        ] if select.eq_ignore_ascii_case("SELECT") && key == again => key.clone(),
        _ => return Ok(None),
    };
    let mut expressions = Vec::new();
    let mut end = 2;
    while let [
        Token::Comma,
        Token::Word(name),
        op @ (Token::Plus | Token::Minus),
        Token::Number(number),
        ..,
    ] = &tokens[end..]
    {
        if name != &key {
            break;
        }
        let amount = number.parse::<i64>().map_err(|_| {
            ParseError("grouped arithmetic requires a non-negative Int64 offset".into())
        })?;
        if amount < 0 {
            return Err(ParseError(
                "grouped arithmetic requires a non-negative Int64 offset".into(),
            ));
        }
        let subtract = matches!(op, Token::Minus);
        expressions.push(GroupedAffineKey {
            label: format!("{key} {} {amount}", if subtract { "-" } else { "+" }),
            offset: if subtract { -amount } else { amount },
        });
        if expressions.len() > 7 {
            return Err(ParseError("GROUP BY supports at most 8 keys".into()));
        }
        end += 4;
    }
    if !matches!(tokens.get(end), Some(Token::Comma))
        || !matches!(tokens.get(end+1), Some(Token::Word(name)) if name.eq_ignore_ascii_case("COUNT"))
    {
        return Ok(None);
    }
    let mut depth = 0usize;
    let mut group = None;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word)
                if depth == 0
                    && word.eq_ignore_ascii_case("GROUP")
                    && matches!(tokens.get(i+1), Some(Token::Word(by)) if by.eq_ignore_ascii_case("BY")) =>
            {
                group = Some(i);
                break;
            }
            _ => {}
        }
    }
    let Some(group) = group else { return Ok(None) };
    if !matches!(tokens.get(group+2), Some(Token::Word(word)) if word == &key) {
        return Ok(None);
    }
    let mut group_end = group + 3;
    for expression in &expressions {
        let expected_op = if expression.label.contains(" - ") {
            Token::Minus
        } else {
            Token::Plus
        };
        match tokens.get(group_end..group_end + 4) {
            Some([Token::Comma, Token::Word(name), op, Token::Number(number)])
                if name == &key
                    && *op == expected_op
                    && number.parse::<i64>().ok() == Some(expression.offset.abs()) => {}
            _ => return Ok(None),
        }
        group_end += 4;
    }
    if matches!(tokens.get(group_end), Some(Token::Comma)) {
        return Ok(None);
    }
    if has_positional_order(&tokens, group_end) {
        return Err(ParseError(
            "positional ordering with grouped expressions is unsupported".into(),
        ));
    }
    tokens.drain(group + 3..group_end);
    tokens.drain(2..end);
    let mut query = parse_select_tokens(tokens)?;
    let grouped_count = match &query.projection {
        Projection::GroupedCount { column } => column == &key,
        Projection::GroupedAggregates { column, items, .. } => {
            column == &key
                && matches!(items.as_slice(), [(crate::AggregateFunction::Count, value)] if value == "*")
        }
        _ => false,
    };
    if !grouped_count {
        return Ok(None);
    }
    query.grouped_affine = expressions;
    Ok(Some(query))
}
