use super::*;

pub(super) fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub(super) fn render_token(token: &Token) -> String {
    match token {
        Token::Word(value) | Token::Number(value) => value.clone(),
        Token::QuotedWord(value) | Token::QuotedAlias(value) => quote_identifier(value),
        Token::String(value) => format!("'{}'", value.replace('\'', "''")),
        Token::Star => "*".into(),
        Token::Slash => "/".into(),
        Token::Percent => "%".into(),
        Token::Plus => "+".into(),
        Token::Minus => "-".into(),
        Token::Comma => ",".into(),
        Token::Dot => ".".into(),
        Token::LParen => "(".into(),
        Token::RParen => ")".into(),
        Token::Semicolon => ";".into(),
        Token::Operator(operator) => match operator {
            crate::Operator::Eq => "=".into(),
            crate::Operator::Ne => "!=".into(),
            crate::Operator::Lt => "<".into(),
            crate::Operator::Le => "<=".into(),
            crate::Operator::Gt => ">".into(),
            crate::Operator::Ge => ">=".into(),
            crate::Operator::Modulo(value) => format!("%{value}"),
            crate::Operator::ModuloNe(value) => format!("%{value} !="),
        },
    }
}

pub(super) fn render_tokens(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(render_token)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn identifier_token(token: &Token) -> bool {
    matches!(token, Token::Word(_) | Token::QuotedWord(_))
}

pub(super) fn is_column_tokens(tokens: &[Token]) -> bool {
    matches!(tokens, [name] if identifier_token(name))
        || matches!(tokens, [relation, Token::Dot, name] if identifier_token(relation) && identifier_token(name))
}

pub(super) fn has_column_offset_equality(tokens: &[Token]) -> bool {
    for left_len in [1, 3] {
        for right_len in [1, 3] {
            let width = left_len + 1 + right_len + 2;
            for comparison in tokens.windows(width) {
                let right_start = left_len + 1;
                if is_column_tokens(&comparison[..left_len])
                    && comparison[left_len] == Token::Operator(crate::Operator::Eq)
                    && is_column_tokens(&comparison[right_start..right_start + right_len])
                    && matches!(
                        comparison[right_start + right_len],
                        Token::Plus | Token::Minus
                    )
                    && matches!(&comparison[right_start + right_len + 1], Token::Number(value) if value.bytes().all(|byte| byte.is_ascii_digit()))
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Use a bounded CROSS JOIN for a two-source offset equality that the key
/// planner cannot normalize into a hash key. The WHERE expression still checks
/// every candidate pair, and normal JOIN result limits remain in force.
pub(super) fn rewrite_offset_comma_join(sql: &str) -> Result<Option<String>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut from = None;
    let mut where_clause = None;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("FROM") => {
                from = Some(index);
            }
            Token::Word(word)
                if depth == 0 && from.is_some() && word.eq_ignore_ascii_case("WHERE") =>
            {
                where_clause = Some(index);
                break;
            }
            _ => {}
        }
    }
    let (Some(from), Some(where_clause)) = (from, where_clause) else {
        return Ok(None);
    };
    if !has_column_offset_equality(&tokens[where_clause + 1..]) {
        return Ok(None);
    }

    depth = 0;
    let mut comma = None;
    for (index, token) in tokens.iter().enumerate().take(where_clause).skip(from + 1) {
        match token {
            Token::LParen => return Ok(None),
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                if comma.replace(index).is_some() {
                    return Ok(None);
                }
            }
            Token::Word(word)
                if depth == 0
                    && ["JOIN", "CROSS", "LEFT", "RIGHT", "FULL"]
                        .iter()
                        .any(|keyword| word.eq_ignore_ascii_case(keyword)) =>
            {
                return Ok(None);
            }
            _ => {}
        }
    }
    let Some(comma) = comma else {
        return Ok(None);
    };
    let mut rewritten = tokens[..comma].to_vec();
    rewritten.extend([Token::Word("CROSS".into()), Token::Word("JOIN".into())]);
    rewritten.extend_from_slice(&tokens[comma + 1..]);
    Ok(Some(render_tokens(&rewritten)))
}

