//! Parse grouped REGEXP_REPLACE with AVG(length), COUNT and MIN.
use crate::lexer::Token;
use crate::{AggregateFunction, Operator, ParseError, Projection, Select, parse_select_tokens};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedRegex {
    pub column: String,
    pub pattern: String,
    pub replacement: String,
    pub key_label: String,
    pub average_label: String,
    pub count_label: String,
    pub threshold: u64,
}
pub(super) fn parse(mut tokens: Vec<Token>) -> Result<Option<Select>, ParseError> {
    let (column, pattern, replacement, key_label, average_label, count_label) =
        match tokens.as_slice() {
            [
                Token::Word(select),
                Token::Word(function),
                Token::LParen,
                Token::Word(column),
                Token::Comma,
                Token::String(pattern),
                Token::Comma,
                Token::String(replacement),
                Token::RParen,
                Token::Word(as1),
                Token::Word(key_label),
                Token::Comma,
                Token::Word(avg),
                Token::LParen,
                Token::Word(length),
                Token::LParen,
                Token::Word(length_col),
                Token::RParen,
                Token::RParen,
                Token::Word(as2),
                Token::Word(average_label),
                Token::Comma,
                Token::Word(count),
                Token::LParen,
                Token::Star,
                Token::RParen,
                Token::Word(as3),
                Token::Word(count_label),
                Token::Comma,
                Token::Word(min),
                Token::LParen,
                Token::Word(min_col),
                Token::RParen,
                Token::Word(from),
                ..,
            ] if select.eq_ignore_ascii_case("SELECT")
                && function.eq_ignore_ascii_case("REGEXP_REPLACE")
                && as1.eq_ignore_ascii_case("AS")
                && avg.eq_ignore_ascii_case("AVG")
                && length.eq_ignore_ascii_case("LENGTH")
                && length_col == column
                && as2.eq_ignore_ascii_case("AS")
                && count.eq_ignore_ascii_case("COUNT")
                && as3.eq_ignore_ascii_case("AS")
                && min.eq_ignore_ascii_case("MIN")
                && min_col == column
                && from.eq_ignore_ascii_case("FROM") =>
            {
                (
                    column.clone(),
                    pattern.clone(),
                    replacement.clone(),
                    key_label.clone(),
                    average_label.clone(),
                    count_label.clone(),
                )
            }
            _ => return Ok(None),
        };
    if pattern.len() > 4096 || replacement.len() > 4096 {
        return Err(ParseError(
            "regular expression or replacement exceeds 4096 bytes".into(),
        ));
    }
    if key_label.eq_ignore_ascii_case(&average_label)
        || key_label.eq_ignore_ascii_case(&count_label)
        || average_label.eq_ignore_ascii_case(&count_label)
    {
        return Err(ParseError("ambiguous REGEXP output alias".into()));
    }
    let mut group = None;
    let mut having = None;
    let mut order = None;
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
                group = Some(i)
            }
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("HAVING") => {
                having = Some(i)
            }
            Token::Word(word)
                if depth == 0
                    && word.eq_ignore_ascii_case("ORDER")
                    && matches!(tokens.get(i+1), Some(Token::Word(by)) if by.eq_ignore_ascii_case("BY")) =>
            {
                order = Some(i)
            }
            _ => {}
        }
    }
    let (Some(group), Some(having), Some(order)) = (group, having, order) else {
        return Ok(None);
    };
    if !matches!(tokens.get(group+2), Some(Token::Word(alias)) if alias.eq_ignore_ascii_case(&key_label))
        || having != group + 3
    {
        return Ok(None);
    }
    let threshold = match tokens.get(having..order) {
        Some(
            [
                Token::Word(h),
                Token::Word(c),
                Token::LParen,
                Token::Star,
                Token::RParen,
                Token::Operator(Operator::Gt),
                Token::Number(number),
            ],
        ) if h.eq_ignore_ascii_case("HAVING") && c.eq_ignore_ascii_case("COUNT") => number
            .parse::<u64>()
            .map_err(|_| ParseError("invalid REGEXP group threshold".into()))?,
        _ => return Ok(None),
    };
    if !matches!(tokens.get(order+2), Some(Token::Word(alias)) if alias.eq_ignore_ascii_case(&average_label))
    {
        return Ok(None);
    }
    tokens[order + 2] = Token::Word(column.clone());
    tokens.drain(having..order);
    tokens[group + 2] = Token::Word(column.clone());
    tokens.splice(
        1..33,
        [
            Token::Word(column.clone()),
            Token::Comma,
            Token::Word("COUNT".into()),
            Token::LParen,
            Token::Star,
            Token::RParen,
            Token::Word("AS".into()),
            Token::Word(count_label.clone()),
        ],
    );
    let mut select = parse_select_tokens(tokens)?;
    let valid = matches!(&select.projection, Projection::GroupedAggregates { column: name, items, .. } if name == &column && matches!(items.as_slice(), [(AggregateFunction::Count, value)] if value == "*"));
    if !valid || select.having.is_some() || !select.order_by_extra.is_empty() {
        return Err(ParseError("REGEXP grouping supports one order key".into()));
    }
    if select
        .order_by
        .as_ref()
        .is_none_or(|order| order.column != column)
    {
        return Err(ParseError("REGEXP grouping orders by its average".into()));
    }
    select.grouped_regex = Some(GroupedRegex {
        column,
        pattern,
        replacement,
        key_label,
        average_label,
        count_label,
        threshold,
    });
    Ok(Some(select))
}
