use super::*;

pub(super) fn is_ascii_digit_at(bytes: &[u8], index: usize) -> bool {
    matches!(bytes.get(index), Some(b'0'..=b'9'))
}

pub(super) fn advance_ascii_digits(bytes: &[u8], index: &mut usize) {
    while is_ascii_digit_at(bytes, *index) {
        *index += 1;
    }
}

pub(super) fn require_digit(
    bytes: &[u8],
    index: usize,
    message: &'static str,
) -> Result<(), ParseError> {
    if !is_ascii_digit_at(bytes, index) {
        return Err(ParseError(message.into()));
    }
    Ok(())
}

fn scan_exponent(bytes: &[u8], index: &mut usize) -> Result<(), ParseError> {
    if matches!(bytes.get(*index), Some(b'e' | b'E')) {
        *index += 1;
        if matches!(bytes.get(*index), Some(b'+' | b'-')) {
            *index += 1;
        }
        require_digit(bytes, *index, "expected digits in exponent")?;
        advance_ascii_digits(bytes, index);
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
    let has_whole_digits = *index > start + usize::from(bytes[start] == b'-');
    if bytes.get(*index) == Some(&b'.') {
        *index += 1;
        if !has_whole_digits {
            require_digit(bytes, *index, "expected digits after decimal point")?;
        }
        advance_ascii_digits(bytes, index);
    }
    scan_exponent(bytes, index)?;
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

/// Parse quoted identifiers without treating reserved words as SQL keywords.
/// Database identifiers must keep the same bounded ASCII rules as unquoted
/// names so quoted DDL cannot bypass storage path validation.
pub(super) fn scan_quoted_identifier(sql: &str, index: &mut usize) -> Result<Token, ParseError> {
    let bytes = sql.as_bytes();
    let delimiter = bytes[*index];
    let mut cursor = *index + 1;
    let mut decoded = Vec::new();
    loop {
        let Some(&byte) = bytes.get(cursor) else {
            return Err(ParseError("unterminated quoted identifier".into()));
        };
        if byte == delimiter && bytes.get(cursor + 1) != Some(&delimiter) {
            *index = cursor + 1;
            break;
        }
        // Both doubled quotes and backslash-escaped delimiters are accepted,
        // then validated against the safe identifier grammar below.
        if (byte == delimiter || byte == b'\\')
            && bytes.get(cursor + 1) == Some(&delimiter)
        {
            decoded.push(delimiter);
            cursor += 2;
        } else {
            decoded.push(byte);
            cursor += 1;
        }
        if decoded.len() > 128 {
            return Err(ParseError("quoted identifier exceeds 128 bytes".into()));
        }
    }
    let mut characters = decoded.iter().copied();
    let valid = matches!(
        characters.next(),
        Some(b'a'..=b'z' | b'A'..=b'Z' | b'_')
    ) && characters.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !valid {
        return Err(ParseError(
            "quoted identifiers require a simple ASCII identifier".into(),
        ));
    }
    Ok(Token::QuotedWord(
        String::from_utf8(decoded).expect("validated ASCII quoted identifier"),
    ))
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
    match tokens.last() {
        Some(Token::Number(_) | Token::RParen) => true,
        Some(Token::QuotedWord(_)) => true,
        Some(Token::Word(word)) => !["SELECT", "BETWEEN", "AND", "OR", "WHERE", "NOT", "IN"]
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword)),
        _ => false,
    }
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

#[cfg(test)]
mod exponent_tests {
    use super::*;

    #[test]
    fn accepts_trailing_dot_decimal_literals_without_accepting_bare_dot() {
        for (input, expected) in [
            ("1.", "1."), ("-12.", "-12."), ("1.e2", "1.e2"),
            (".5", ".5"), ("-.5", "-.5"),
        ] {
            let mut index = 0;
            assert_eq!(scan_number(input, &mut index).unwrap(),
                Token::Number(expected.into()));
            assert_eq!(index, input.len());
        }
        assert!(scan_number("-.",&mut 0).is_err());
    }

    #[test]
    fn scans_signed_exponents_without_splitting_the_number() {
        let mut index = 0;
        assert_eq!(
            scan_number("2.5E-3", &mut index).unwrap(),
            Token::Number("2.5E-3".into())
        );
        assert_eq!(index, 6);
        for invalid in ["2e", "2e+", "2e-"] {
            assert!(scan_number(invalid, &mut 0).is_err());
        }
    }
}
