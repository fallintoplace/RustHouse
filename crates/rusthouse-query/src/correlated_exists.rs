//! Parse a bounded correlated EXISTS that can be evaluated as a semi-join.
use crate::{ParseError, Projection, Select, Source, Token, parse_select_tokens, tokenize};

#[derive(Debug, Clone)]
pub struct CorrelatedExists {
    pub outer: Select,
    pub inner: Select,
    pub negated: bool,
}

fn supported_inner_select(inner: &Select) -> bool {
    matches!(&inner.source, Some(Source::Table(_)))
        && matches!(&inner.projection, Projection::All)
        && !inner.distinct
        && inner.rename.is_none()
        && inner.or_predicates.is_empty()
        && inner.order_by.is_none()
        && inner.limit.is_none()
        && inner.offset.is_none()
        && inner.having.is_none()
}

fn closing_exists_index(tokens: &[Token], start: usize) -> Result<usize, ParseError> {
    let mut nesting = 1usize;
    let mut end = start + 2;
    while end < tokens.len() && nesting != 0 {
        match tokens[end] {
            Token::LParen => nesting += 1,
            Token::RParen => nesting -= 1,
            _ => {}
        }
        end += 1;
    }
    if nesting != 0 {
        return Err(ParseError("EXISTS requires a closing ')'".into()));
    }
    Ok(end)
}

/// Recognize a conjunctive EXISTS in a WHERE clause.
/// Other forms continue to use the regular parser and its diagnostics.
pub fn parse_correlated_exists(sql: &str) -> Result<Option<CorrelatedExists>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut where_start = None;
    let mut found = None;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(w)
                if depth == 0 && w.eq_ignore_ascii_case("WHERE") && where_start.is_none() =>
            {
                where_start = Some(i)
            }
            Token::Word(w)
                if depth == 0 && where_start.is_some() && w.eq_ignore_ascii_case("EXISTS") =>
            {
                found = match found {
                    Some(_) => {
                        return Err(ParseError("only one correlated EXISTS is supported".into()));
                    }
                    None => Some(i),
                };
            }
            _ => {}
        }
    }
    let Some(start) = found else { return Ok(None) };
    let where_start = where_start.expect("EXISTS follows WHERE");
    let negated =
        start > 0 && matches!(&tokens[start - 1], Token::Word(w) if w.eq_ignore_ascii_case("NOT"));
    let operator_start = start - usize::from(negated);
    let first = operator_start == where_start + 1;
    if !first
        && !matches!(&tokens[operator_start - 1], Token::Word(w) if w.eq_ignore_ascii_case("AND"))
    {
        return Err(ParseError(
            "correlated EXISTS requires a conjunctive WHERE".into(),
        ));
    }
    if tokens.get(start + 1) != Some(&Token::LParen) {
        return Err(ParseError("EXISTS requires a parenthesized SELECT".into()));
    }
    let end = closing_exists_index(&tokens, start)?;
    // Pulling EXISTS out of an OR changes SQL precedence. Refuse it explicitly.
    if tokens[where_start + 1..if first {
        operator_start
    } else {
        operator_start - 1
    }]
        .iter()
        .chain(tokens[end..].iter())
        .any(|t| matches!(t, Token::Word(w) if w.eq_ignore_ascii_case("OR")))
    {
        return Err(ParseError(
            "correlated EXISTS with OR is not yet supported".into(),
        ));
    }
    let inner = parse_select_tokens(tokens[start + 2..end - 1].to_vec())?;
    if !supported_inner_select(&inner) {
        return Err(ParseError(
            "correlated EXISTS requires a simple SELECT * FROM table".into(),
        ));
    }
    let mut outer_tokens = tokens[..operator_start - 1].to_vec();
    if first && matches!(tokens.get(end), Some(Token::Word(w)) if w.eq_ignore_ascii_case("AND")) {
        outer_tokens.push(tokens[where_start].clone());
        outer_tokens.extend_from_slice(&tokens[end + 1..]);
    } else {
        outer_tokens.extend_from_slice(&tokens[end..]);
    }
    let outer = parse_select_tokens(outer_tokens)?;
    if !matches!(outer.source, Some(Source::Table(_))) {
        return Err(ParseError(
            "correlated EXISTS requires a stored outer table".into(),
        ));
    }
    Ok(Some(CorrelatedExists {
        outer,
        inner,
        negated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_conjunctive_exists_and_rejects_or() {
        let sql = "SELECT count(*) FROM orders WHERE d = 1 AND EXISTS (SELECT * FROM lines WHERE lines_id = orders_id AND a < b)";
        let q = parse_correlated_exists(sql).unwrap().unwrap();
        assert_eq!(q.outer.predicates.len(), 1);
        assert_eq!(q.inner.predicates.len(), 2);
        assert!(parse_correlated_exists("SELECT count(*) FROM orders WHERE d = 1 OR d = 2 AND EXISTS (SELECT * FROM lines WHERE a = b)").is_err());
    }
}
