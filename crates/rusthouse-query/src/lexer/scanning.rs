use super::*;

pub(super) fn is_ascii_digit_at(bytes: &[u8], index: usize) -> bool {
    matches!(bytes.get(index), Some(b'0'..=b'9'))
}

pub(super) fn advance_ascii_digits(bytes: &[u8], index: &mut usize) {
    while is_ascii_digit_at(bytes, *index) {
        *index += 1;
    }
}

pub(super) fn require_digit(bytes: &[u8], index: usize, message: &'static str) -> Result<(), ParseError> {
    if !is_ascii_digit_at(bytes, index) {
        return Err(ParseError(message.into()));
    }
    Ok(())
}

pub(super) fn scan_number(sql: &str, index: &mut usize) -> Result<Token, ParseError> {
    let bytes = sql.as_bytes();
    let start = *index;
    if bytes[*index] == b'-' {
        *index += 1;
        if bytes.get(*index) != Some(&b'.') {
            require_digit(bytes, *index, "expected digits after '-'")?;
        }
    }
    advance_ascii_digits(bytes, index);
    if bytes.get(*index) == Some(&b'.') {
        *index += 1;
        require_digit(bytes, *index, "expected digits after decimal point")?;
        advance_ascii_digits(bytes, index);
    }
    Ok(Token::Number(sql[start..*index].into()))
}

pub(super) fn scan_string(sql: &str, index: &mut usize) -> Result<Token, ParseError> {
    let bytes = sql.as_bytes();
    let mut cursor = *index;
    cursor += 1;
    let mut value = String::new();
    let mut start = cursor;
    while cursor < bytes.len() {
        let escaped = match (bytes[cursor], bytes.get(cursor + 1)) {
            (b'\\', Some(b'\'' | b'\\')) => Some(bytes[cursor + 1]),
            (b'\'', Some(b'\'')) => Some(b'\''),
            _ => None,
        };
        if let Some(escaped) = escaped {
            value.push_str(&sql[start..cursor]);
            value.push(escaped as char);
            cursor += 2;
            start = cursor;
        } else if bytes[cursor] == b'\'' {
            value.push_str(&sql[start..cursor]);
            *index = cursor + 1;
            return Ok(Token::String(value));
        } else {
            cursor += 1;
        }
    }
    Err(ParseError("unterminated string literal".into()))
}

pub(super) fn scan_word(sql: &str, index: &mut usize) -> Token {
    let bytes = sql.as_bytes();
    let start = *index;
    *index += 1;
    while matches!(
        bytes.get(*index),
        Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
    ) {
        *index += 1;
    }
    Token::Word(sql[start..*index].into())
}

pub(super) fn scan_comparison(bytes: &[u8], index: &mut usize) -> Operator {
    let (operator, width) = match (bytes[*index], bytes.get(*index + 1)) {
        (b'<', Some(b'=')) => (Operator::Le, 2),
        (b'<', Some(b'>')) | (b'!', Some(b'=')) => (Operator::Ne, 2),
        (b'>', Some(b'=')) => (Operator::Ge, 2),
        (b'=', _) => (Operator::Eq, 1),
        (b'<', _) => (Operator::Lt, 1),
        (b'>', _) => (Operator::Gt, 1),
        _ => unreachable!("only comparison operator starters reach this helper"),
    };
    *index += width;
    operator
}

pub(super) fn follows_binary_minus_context(tokens: &[Token]) -> bool {
    matches!(tokens.last(), Some(Token::Number(_) | Token::RParen))
        || matches!(tokens.last(), Some(Token::Word(word)) if !word.eq_ignore_ascii_case("SELECT"))
}

pub(super) fn punctuation_token(byte: u8) -> Token {
    match byte {
        b'*' => Token::Star,
        b'/' => Token::Slash,
        b'%' => Token::Percent,
        b'+' => Token::Plus,
        b',' => Token::Comma,
        b'.' => Token::Dot,
        b'(' => Token::LParen,
        b')' => Token::RParen,
        b';' => Token::Semicolon,
        _ => unreachable!("only supported punctuation reaches this helper"),
    }
}
