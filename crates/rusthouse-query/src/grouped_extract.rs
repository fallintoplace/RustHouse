//! Parse a three-key grouped COUNT with EXTRACT(minute FROM DateTime).
use crate::lexer::Token;
use crate::{AggregateFunction, ParseError, Projection, Select, parse_select_tokens};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedExtractMinute {
    pub column: String,
    pub label: String,
}
pub(super) fn parse(mut tokens: Vec<Token>) -> Result<Option<Select>, ParseError> {
    let (first, column, label, third) = match tokens.as_slice() {
        [
            Token::Word(select),
            Token::Word(first),
            Token::Comma,
            Token::Word(extract),
            Token::LParen,
            Token::Word(unit),
            Token::Word(from),
            Token::Word(column),
            Token::RParen,
            Token::Word(as_word),
            Token::Word(label),
            Token::Comma,
            Token::Word(third),
            Token::Comma,
            Token::Word(count),
            ..,
        ] if select.eq_ignore_ascii_case("SELECT")
            && extract.eq_ignore_ascii_case("EXTRACT")
            && unit.eq_ignore_ascii_case("minute")
            && from.eq_ignore_ascii_case("FROM")
            && as_word.eq_ignore_ascii_case("AS")
            && count.eq_ignore_ascii_case("COUNT") =>
        {
            (first.clone(), column.clone(), label.clone(), third.clone())
        }
        _ => return Ok(None),
    };
    if first == column || third == column || first == third || label == first || label == third {
        return Err(ParseError("ambiguous extracted group key".into()));
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
                group = Some(i + 2);
                break;
            }
            _ => {}
        }
    }
    let Some(group) = group else { return Ok(None) };
    if !matches!(tokens.get(group..group+5), Some([Token::Word(a), Token::Comma, Token::Word(b), Token::Comma, Token::Word(c)]) if a == &first && b == &label && c == &third)
        || matches!(tokens.get(group + 5), Some(Token::Comma))
    {
        return Ok(None);
    }
    tokens[group + 2] = Token::Word(column.clone());
    tokens.splice(3..11, [Token::Word(column.clone())]);
    let mut select = parse_select_tokens(tokens)?;
    if label.eq_ignore_ascii_case("count")
        || matches!(&select.projection, Projection::GroupedMultiAggregates { aliases, .. } if aliases.iter().flatten().any(|alias| alias.eq_ignore_ascii_case(&label)))
    {
        return Err(ParseError(
            "EXTRACT alias conflicts with the COUNT output".into(),
        ));
    }
    let valid = match &select.projection {
        Projection::GroupedMultiCount { columns } => {
            columns == &[first.clone(), column.clone(), third.clone()]
        }
        Projection::GroupedMultiAggregates { columns, items, .. } => {
            columns == &[first, column.clone(), third]
                && matches!(items.as_slice(), [(AggregateFunction::Count, value)] if value == "*")
        }
        _ => false,
    };
    if !valid || select.having.is_some() || !select.order_by_extra.is_empty() {
        return Err(ParseError(
            "EXTRACT minute grouping supports one COUNT and one sort key".into(),
        ));
    }
    if select
        .order_by
        .as_ref()
        .is_some_and(|order| order.column != "count" && order.column != "#aggregate0")
    {
        return Err(ParseError(
            "EXTRACT minute grouping orders only by COUNT".into(),
        ));
    }
    select.grouped_extract_minute = Some(GroupedExtractMinute { column, label });
    Ok(Some(select))
}
