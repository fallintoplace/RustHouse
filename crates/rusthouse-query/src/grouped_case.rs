//! Parse grouped COUNT with a two-flag string CASE key.
use crate::lexer::Token;
use crate::{AggregateFunction, Operator, ParseError, Projection, Select, parse_select_tokens};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedCaseKey {
    pub first_flag: String,
    pub second_flag: String,
    pub case_label: String,
    pub destination_label: String,
}
fn matches_case_projection(projection: &Projection, expected: &[&str]) -> bool {
    match projection {
        Projection::GroupedMultiCount { columns } => {
            columns.iter().map(String::as_str).eq(expected.iter().copied())
        }
        Projection::GroupedMultiAggregates { columns, items, .. } => {
            columns.iter().map(String::as_str).eq(expected.iter().copied())
                && matches!(items.as_slice(), [(AggregateFunction::Count, value)] if value == "*")
        }
        _ => false,
    }
}

fn has_duplicate_case_keys(names: &[&String]) -> bool {
    names
        .iter()
        .enumerate()
        .any(|(index, name)| names[..index].contains(name))
}

pub(super) fn parse(mut tokens: Vec<Token>) -> Result<Option<Select>, ParseError> {
    let (first, flag1, flag2, source, case_label, destination, destination_label) =
        match tokens.as_slice() {
            [
                Token::Word(select),
                Token::Word(first),
                Token::Comma,
                Token::Word(flag1),
                Token::Comma,
                Token::Word(flag2),
                Token::Comma,
                Token::Word(case),
                Token::Word(when),
                Token::LParen,
                Token::Word(test1),
                Token::Operator(Operator::Eq),
                Token::Number(zero1),
                Token::Word(and),
                Token::Word(test2),
                Token::Operator(Operator::Eq),
                Token::Number(zero2),
                Token::RParen,
                Token::Word(then),
                Token::Word(source),
                Token::Word(otherwise),
                Token::String(empty),
                Token::Word(end),
                Token::Word(as1),
                Token::Word(case_label),
                Token::Comma,
                Token::Word(destination),
                Token::Word(as2),
                Token::Word(destination_label),
                Token::Comma,
                Token::Word(count),
                ..,
            ] if select.eq_ignore_ascii_case("SELECT")
                && case.eq_ignore_ascii_case("CASE")
                && when.eq_ignore_ascii_case("WHEN")
                && test1 == flag1
                && test2 == flag2
                && zero1 == "0"
                && zero2 == "0"
                && and.eq_ignore_ascii_case("AND")
                && then.eq_ignore_ascii_case("THEN")
                && otherwise.eq_ignore_ascii_case("ELSE")
                && empty.is_empty()
                && end.eq_ignore_ascii_case("END")
                && as1.eq_ignore_ascii_case("AS")
                && as2.eq_ignore_ascii_case("AS")
                && count.eq_ignore_ascii_case("COUNT") =>
            {
                (
                    first.clone(),
                    flag1.clone(),
                    flag2.clone(),
                    source.clone(),
                    case_label.clone(),
                    destination.clone(),
                    destination_label.clone(),
                )
            }
            _ => return Ok(None),
        };
    let names = [&first, &flag1, &flag2, &source, &destination];
    if has_duplicate_case_keys(&names) || case_label.eq_ignore_ascii_case(&destination_label) {
        return Err(ParseError("ambiguous CASE grouping key".into()));
    }
    let mut group = None;
    let mut depth = 0usize;
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
    if !matches!(tokens.get(group..group+9), Some([Token::Word(a), Token::Comma, Token::Word(b), Token::Comma, Token::Word(c), Token::Comma, Token::Word(d), Token::Comma, Token::Word(e)]) if a == &first && b == &flag1 && c == &flag2 && d == &case_label && e == &destination_label)
        || matches!(tokens.get(group + 9), Some(Token::Comma))
    {
        return Ok(None);
    }
    tokens[group + 6] = Token::Word(source.clone());
    tokens[group + 8] = Token::Word(destination.clone());
    tokens.drain(27..29);
    tokens.splice(7..25, [Token::Word(source.clone())]);
    let mut select = parse_select_tokens(tokens)?;
    let valid = matches_case_projection(
        &select.projection,
        &[&first, &flag1, &flag2, &source, &destination],
    );
    if case_label.eq_ignore_ascii_case("count")
        || destination_label.eq_ignore_ascii_case("count")
        || matches!(&select.projection, Projection::GroupedMultiAggregates { aliases, .. } if aliases.iter().flatten().any(|alias| alias.eq_ignore_ascii_case(&case_label) || alias.eq_ignore_ascii_case(&destination_label)))
    {
        return Err(ParseError("CASE alias conflicts with COUNT output".into()));
    }
    if !valid || select.having.is_some() || !select.order_by_extra.is_empty() {
        return Err(ParseError(
            "CASE grouping supports one COUNT and one sort key".into(),
        ));
    }
    if select
        .order_by
        .as_ref()
        .is_some_and(|order| order.column != "count" && order.column != "#aggregate0")
    {
        return Err(ParseError("CASE grouping orders only by COUNT".into()));
    }
    select.grouped_case_key = Some(GroupedCaseKey {
        first_flag: flag1,
        second_flag: flag2,
        case_label,
        destination_label,
    });
    Ok(Some(select))
}
