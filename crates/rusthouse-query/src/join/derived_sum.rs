//! Flatten a bounded grouped SUM over a simple joined derived table.
use super::{JoinSelect, ParseError, Parser, Token, parse_join_resolved, projected, tokenize};

fn keyword(token: &Token, name: &str) -> bool {
    matches!(token, Token::Word(word) if word.eq_ignore_ascii_case(name))
}

fn top_from(tokens: &[Token]) -> Option<usize> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && keyword(token, "FROM") => return Some(index),
            _ => {}
        }
    }
    None
}

/// Flatten a grouped SUM over two or three projected keys and one arithmetic expression.
/// Other derived-table shapes are left to their own parser.
pub fn parse_derived_join_sum_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<JoinSelect>, ParseError> {
    if let Some(plan) = super::derived_ratio::parse(sql, &mut resolve)? {
        return Ok(Some(plan));
    }
    let tokens = tokenize(sql)?;
    let Some(from) = top_from(&tokens) else {
        return Ok(None);
    };
    if tokens.get(from + 1) != Some(&Token::LParen)
        || !tokens
            .get(from + 2)
            .is_some_and(|token| keyword(token, "SELECT"))
    {
        return Ok(None);
    }
    let mut outer = Parser {
        tokens: tokens[..from].to_vec(),
        index: 0,
    };
    if !outer.keyword("SELECT") {
        return Ok(None);
    }
    let mut keys = Vec::new();
    loop {
        if matches!(outer.tokens.get(outer.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SUM"))
            && outer.tokens.get(outer.index + 1) == Some(&Token::LParen)
        {
            outer.keyword("SUM");
            break;
        }
        if keys.len() == 3 {
            return Ok(None);
        }
        let key = match outer.bounded_word() {
            Ok(word) => word,
            Err(_) => return Ok(None),
        };
        keys.push(key);
        if !outer.take(Token::Comma) {
            return Ok(None);
        }
    }
    if !(2..=3).contains(&keys.len()) {
        return Ok(None);
    }
    if !outer.take(Token::LParen) {
        return Err(ParseError("derived SUM requires '('".into()));
    }
    let value = outer.bounded_word()?;
    if !outer.take(Token::RParen) || !outer.keyword("AS") {
        return Err(ParseError(
            "derived SUM requires one column and alias".into(),
        ));
    }
    let aggregate_alias = outer.bounded_word()?;
    if outer.index != outer.tokens.len()
        || keys.iter().enumerate().any(|(index, key)| {
            keys[..index]
                .iter()
                .any(|other| key.eq_ignore_ascii_case(other))
                || key.eq_ignore_ascii_case(&value)
                || key.eq_ignore_ascii_case(&aggregate_alias)
        })
    {
        return Err(ParseError("invalid derived SUM output columns".into()));
    }
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
        return Err(ParseError("derived SUM requires closing ')'".into()));
    }
    let inner = &tokens[from + 2..end - 1];
    let inner_from =
        top_from(inner).ok_or_else(|| ParseError("derived SUM requires FROM".into()))?;
    let mut parts: Vec<&[Token]> = Vec::new();
    let mut start = 1;
    let mut level = 0usize;
    for (index, token) in inner.iter().enumerate().take(inner_from).skip(1) {
        match token {
            Token::LParen => level += 1,
            Token::RParen => level = level.saturating_sub(1),
            Token::Comma if level == 0 => {
                parts.push(&inner[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..inner_from]);
    if parts.len() != keys.len() + 1 {
        return Err(ParseError(
            "derived SUM projected column count differs".into(),
        ));
    }
    let mut expressions = Vec::new();
    for (part, expected) in parts.iter().zip(keys.iter().chain(std::iter::once(&value))) {
        if part.len() < 3
            || !keyword(&part[part.len() - 2], "AS")
            || !matches!(&part[part.len()-1], Token::Word(name) | Token::QuotedWord(name) if name.eq_ignore_ascii_case(expected))
        {
            return Err(ParseError(
                "derived SUM projection aliases must match outer columns".into(),
            ));
        }
        expressions.push(&part[..part.len() - 2]);
    }
    for expression in &expressions[..keys.len() - 1] {
        let mut key = Parser {
            tokens: expression.to_vec(),
            index: 0,
        };
        let column = projected(&mut key)?;
        if column.name == "*" || key.index != key.tokens.len() {
            return Err(ParseError("derived SUM group key must be a column".into()));
        }
    }
    let mut year = Parser {
        tokens: expressions[keys.len() - 1].to_vec(),
        index: 0,
    };
    if !year.keyword("EXTRACT")
        || !year.take(Token::LParen)
        || !year.keyword("YEAR")
        || !year.keyword("FROM")
    {
        return Err(ParseError("derived SUM last key must extract YEAR".into()));
    }
    let date = projected(&mut year)?;
    if date.name == "*" || !year.take(Token::RParen) || year.index != year.tokens.len() {
        return Err(ParseError("invalid derived SUM year key".into()));
    }
    // An inner grouping, subquery, ordering, or limit cannot be moved across aggregation.
    if inner[inner_from + 1..].iter().any(|token| matches!(token, Token::Word(word) if ["SELECT", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET"].iter().any(|name| word.eq_ignore_ascii_case(name)))) {
        return Err(ParseError("unsupported derived SUM input".into()));
    }
    let mut suffix = Parser {
        tokens: tokens[end..].to_vec(),
        index: 0,
    };
    suffix.keyword("AS");
    suffix.bounded_word()?;
    let suffix_start = suffix.index;
    if !suffix.keyword("GROUP") || !suffix.keyword("BY") {
        return Err(ParseError("derived SUM requires GROUP BY".into()));
    }
    let mut groups = Vec::new();
    loop {
        if groups.len() == keys.len() {
            return Err(ParseError("derived SUM has too many group keys".into()));
        }
        let name = suffix.bounded_word()?;
        if !keys.iter().any(|key| key.eq_ignore_ascii_case(&name))
            || groups
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&name))
        {
            return Err(ParseError(
                "derived SUM group keys must match projected keys".into(),
            ));
        }
        groups.push(name);
        if !suffix.take(Token::Comma) {
            break;
        }
    }
    if groups.len() != keys.len() {
        return Err(ParseError(
            "derived SUM group keys must match projected keys".into(),
        ));
    }
    if suffix.keyword("ORDER") {
        if !suffix.keyword("BY") {
            return Err(ParseError("derived SUM ORDER requires BY".into()));
        }
        loop {
            let name = suffix.bounded_word()?;
            if !keys
                .iter()
                .chain(std::iter::once(&aggregate_alias))
                .any(|key| name.eq_ignore_ascii_case(key))
            {
                return Err(ParseError(
                    "derived SUM ORDER BY requires an output name".into(),
                ));
            }
            if !suffix.keyword("DESC") {
                suffix.keyword("ASC");
            }
            if suffix.keyword("NULLS") && !suffix.keyword("FIRST") && !suffix.keyword("LAST") {
                return Err(ParseError("invalid NULL ordering".into()));
            }
            if !suffix.take(Token::Comma) {
                break;
            }
        }
    }
    if suffix.keyword("LIMIT") {
        suffix.number()?;
    }
    if suffix.keyword("OFFSET") {
        suffix.number()?;
    }
    suffix.take(Token::Semicolon);
    if suffix.index != suffix.tokens.len() {
        return Err(ParseError("unsupported derived SUM clause".into()));
    }
    let w = |word: &str| Token::Word(word.to_owned());
    let mut flat = vec![w("SELECT")];
    for part in &parts[..keys.len()] {
        flat.extend_from_slice(part);
        flat.push(Token::Comma);
    }
    flat.extend([w("SUM"), Token::LParen]);
    flat.extend_from_slice(expressions[keys.len()]);
    flat.extend([Token::RParen, w("AS"), w(&aggregate_alias)]);
    flat.extend_from_slice(&inner[inner_from..]);
    flat.extend_from_slice(&tokens[end + suffix_start..]);
    let plan = parse_join_resolved(flat, Some(&mut resolve))?
        .ok_or_else(|| ParseError("derived SUM requires an INNER JOIN".into()))?;
    Ok(Some(plan))
}
