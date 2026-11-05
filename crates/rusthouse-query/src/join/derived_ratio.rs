//! Flatten a grouped formula over a simple joined derived table.
use super::{
    JoinSelect, ParseError, Parser, Token, parse_join_formula, parse_join_resolved, projected,
    tokenize,
};

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

pub(super) fn parse(
    sql: &str,
    resolve: &mut dyn FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<JoinSelect>, ParseError> {
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
    let year_name = match outer.bounded_word() {
        Ok(name) => name,
        Err(_) => return Ok(None),
    };
    if !outer.take(Token::Comma) {
        return Ok(None);
    }
    let metric_start = outer.index;
    let Some((formula, metric_name)) = parse_join_formula(&mut outer)? else {
        return Ok(None);
    };
    if outer.index != outer.tokens.len() || year_name.eq_ignore_ascii_case(&metric_name) {
        return Err(ParseError("invalid derived formula output".into()));
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
        return Err(ParseError("derived formula requires closing ')'".into()));
    }
    let inner = &tokens[from + 2..end - 1];
    let inner_from =
        top_from(inner).ok_or_else(|| ParseError("derived formula requires FROM".into()))?;
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
    if parts.len() != 3 {
        return Err(ParseError(
            "derived formula requires three input columns".into(),
        ));
    }
    let mut aliases = Vec::new();
    let mut expressions = Vec::new();
    for part in &parts {
        if part.len() < 3 || !keyword(&part[part.len() - 2], "AS") {
            return Err(ParseError("derived formula requires input aliases".into()));
        }
        let name = match &part[part.len() - 1] {
            Token::Word(name) | Token::QuotedWord(name) => name.clone(),
            _ => return Err(ParseError("derived formula requires input aliases".into())),
        };
        if aliases
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&name))
        {
            return Err(ParseError("duplicate derived formula alias".into()));
        }
        aliases.push(name);
        expressions.push(&part[..part.len() - 2]);
    }
    if !aliases[0].eq_ignore_ascii_case(&year_name)
        || aliases
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&metric_name))
        || aliases[1..].iter().any(|name| {
            [
                "SUM", "CASE", "WHEN", "THEN", "ELSE", "END", "AS", "AND", "OR", "NOT", "LIKE",
                "ILIKE", "LENGTH", "NULL", "TRUE", "FALSE", "CAST", "FROM", "YEAR", "EXTRACT",
            ]
            .iter()
            .any(|word| name.eq_ignore_ascii_case(word))
        })
    {
        return Err(ParseError("invalid derived formula aliases".into()));
    }
    let mut year = Parser {
        tokens: expressions[0].to_vec(),
        index: 0,
    };
    if !year.keyword("EXTRACT")
        || !year.take(Token::LParen)
        || !year.keyword("YEAR")
        || !year.keyword("FROM")
    {
        return Err(ParseError("derived formula key must extract YEAR".into()));
    }
    let date = projected(&mut year)?;
    if date.name == "*" || !year.take(Token::RParen) || year.index != year.tokens.len() {
        return Err(ParseError("invalid derived formula year key".into()));
    }
    let mut condition = Parser {
        tokens: expressions[2].to_vec(),
        index: 0,
    };
    let column = projected(&mut condition)?;
    if column.name == "*" || condition.index != condition.tokens.len() {
        return Err(ParseError(
            "derived formula condition requires a column".into(),
        ));
    }
    for sum in &formula.sums {
        let mut referenced = Vec::new();
        sum.referenced_columns(&mut referenced);
        if referenced.iter().any(|name| {
            !aliases[1..]
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
        }) {
            return Err(ParseError(
                "derived formula references an unknown column".into(),
            ));
        }
    }
    if inner[inner_from + 1..].iter().any(|token| matches!(token, Token::Word(word) if ["SELECT", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET"].iter().any(|name| word.eq_ignore_ascii_case(name)))) {
        return Err(ParseError("unsupported derived formula input".into()));
    }
    let mut suffix = Parser {
        tokens: tokens[end..].to_vec(),
        index: 0,
    };
    suffix.keyword("AS");
    suffix.bounded_word()?;
    let suffix_start = suffix.index;
    if !suffix.keyword("GROUP")
        || !suffix.keyword("BY")
        || !suffix.bounded_word()?.eq_ignore_ascii_case(&year_name)
    {
        return Err(ParseError(
            "derived formula must group by the year alias".into(),
        ));
    }
    if suffix.keyword("ORDER") {
        if !suffix.keyword("BY") {
            return Err(ParseError("derived formula ORDER requires BY".into()));
        }
        loop {
            let name = suffix.bounded_word()?;
            if !name.eq_ignore_ascii_case(&year_name) && !name.eq_ignore_ascii_case(&metric_name) {
                return Err(ParseError(
                    "derived formula ORDER BY requires an output name".into(),
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
        return Err(ParseError("unsupported derived formula clause".into()));
    }
    let mut flat = vec![Token::Word("SELECT".into())];
    flat.extend_from_slice(parts[0]);
    flat.push(Token::Comma);
    for token in &tokens[metric_start..from] {
        let name = match token {
            Token::Word(name) | Token::QuotedWord(name) => Some(name),
            _ => None,
        };
        if name.is_some_and(|name| name.eq_ignore_ascii_case(&aliases[1])) {
            flat.push(Token::LParen);
            flat.extend_from_slice(expressions[1]);
            flat.push(Token::RParen);
        } else if name.is_some_and(|name| name.eq_ignore_ascii_case(&aliases[2])) {
            flat.extend_from_slice(expressions[2]);
        } else {
            flat.push(token.clone());
        }
    }
    flat.extend_from_slice(&inner[inner_from..]);
    flat.extend_from_slice(&tokens[end + suffix_start..]);
    let plan = parse_join_resolved(flat, Some(resolve), false)?
        .ok_or_else(|| ParseError("derived formula requires an INNER JOIN".into()))?;
    Ok(Some(plan))
}
