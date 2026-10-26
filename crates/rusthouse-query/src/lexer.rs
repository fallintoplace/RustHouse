use crate::{Operator, ParseError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    Word(String),
    Number(String),
    String(String),
    Star,
    Percent,
    Plus,
    Minus,
    Comma,
    Dot,
    LParen,
    RParen,
    Semicolon,
    Operator(Operator),
}

fn advance_ascii_digits(bytes: &[u8], index: &mut usize) {
    while matches!(bytes.get(*index), Some(b'0'..=b'9')) {
        *index += 1;
    }
}

fn scan_number(sql: &str, index: &mut usize) -> Result<Token, ParseError> {
    let bytes = sql.as_bytes();
    let start = *index;
    if bytes[*index] == b'-' {
        *index += 1;
        if !matches!(bytes.get(*index), Some(b'0'..=b'9')) {
            return Err(ParseError("expected digits after '-'".into()));
        }
    }
    advance_ascii_digits(bytes, index);
    if bytes.get(*index) == Some(&b'.') {
        *index += 1;
        if !matches!(bytes.get(*index), Some(b'0'..=b'9')) {
            return Err(ParseError("expected digits after decimal point".into()));
        }
        advance_ascii_digits(bytes, index);
    }
    Ok(Token::Number(sql[start..*index].into()))
}

fn scan_string(sql: &str, index: &mut usize) -> Result<Token, ParseError> {
    let bytes = sql.as_bytes();
    let mut cursor = *index;
    cursor += 1;
    let mut value = String::new();
    let mut start = cursor;
    let mut closed = false;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' && matches!(bytes.get(cursor + 1), Some(b'\'' | b'\\')) {
            value.push_str(&sql[start..cursor]);
            value.push(bytes[cursor + 1] as char);
            cursor += 2;
            start = cursor;
        } else if bytes[cursor] == b'\'' {
            value.push_str(&sql[start..cursor]);
            if bytes.get(cursor + 1) == Some(&b'\'') {
                value.push('\'');
                cursor += 2;
                start = cursor;
            } else {
                cursor += 1;
                closed = true;
                break;
            }
        } else {
            cursor += 1;
        }
    }
    if !closed {
        return Err(ParseError("unterminated string literal".into()));
    }
    *index = cursor;
    Ok(Token::String(value))
}

fn scan_word(sql: &str, index: &mut usize) -> Token {
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

fn scan_comparison(bytes: &[u8], index: &mut usize) -> Operator {
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

pub(super) fn tokenize(sql: &str) -> Result<Vec<Token>, ParseError> {
    if sql.len() > 4096 {
        return Err(ParseError("statement exceeds the 4096-byte limit".into()));
    }
    let bytes = sql.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let token = match byte {
            b' ' | b'\t' | b'\r' | b'\n' => {
                index += 1;
                continue;
            }
            b'\'' => scan_string(sql, &mut index)?,
            b'*' => {
                index += 1;
                Token::Star
            }
            b'%' => {
                index += 1;
                Token::Percent
            }
            b'+' => {
                index += 1;
                Token::Plus
            }
            b'-' if bytes.get(index + 1) == Some(&b'.') => {
                return Err(ParseError("expected digits after '-'".into()));
            }
            b'-' if !matches!(bytes.get(index + 1), Some(b'0'..=b'9')) => {
                index += 1;
                Token::Minus
            }
            b',' => {
                index += 1;
                Token::Comma
            }
            b'.' => {
                index += 1;
                Token::Dot
            }
            b'(' => {
                index += 1;
                Token::LParen
            }
            b')' => {
                index += 1;
                Token::RParen
            }
            b';' => {
                index += 1;
                Token::Semicolon
            }
            b'=' | b'<' | b'>' => Token::Operator(scan_comparison(bytes, &mut index)),
            b'!' if bytes.get(index + 1) == Some(&b'=') => {
                Token::Operator(scan_comparison(bytes, &mut index))
            }
            b'0'..=b'9' | b'-' => scan_number(sql, &mut index)?,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => scan_word(sql, &mut index),
            _ => {
                return Err(ParseError(format!(
                    "unsupported SQL character at byte {index}"
                )));
            }
        };
        tokens.push(token);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_ascii_and_oversized_statements() {
        assert!(tokenize("SELECT é").is_err());
        assert!(tokenize(&" ".repeat(4097)).is_err());
    }

    #[test]
    fn supports_backslash_quoted_strings_without_losing_literal_escapes() {
        assert_eq!(
            tokenize(r"SELECT 'it\'s', 'C:\\tmp', 'a\_b'").unwrap(),
            vec![
                Token::Word("SELECT".into()),
                Token::String("it's".into()),
                Token::Comma,
                Token::String(r"C:\tmp".into()),
                Token::Comma,
                Token::String(r"a\_b".into()),
            ]
        );
    }

    #[test]
    fn distinguishes_signed_literals_and_comparison_tokens() {
        assert_eq!(
            tokenize("SELECT -12 WHERE number != 0").unwrap(),
            vec![
                Token::Word("SELECT".into()),
                Token::Number("-12".into()),
                Token::Word("WHERE".into()),
                Token::Word("number".into()),
                Token::Operator(Operator::Ne),
                Token::Number("0".into()),
            ]
        );
    }

    #[test]
    fn recognizes_all_comparison_spellings_and_decimal_literals() {
        assert_eq!(
            tokenize("<= >= <> != < > = -3.5 2.0").unwrap(),
            vec![
                Token::Operator(Operator::Le),
                Token::Operator(Operator::Ge),
                Token::Operator(Operator::Ne),
                Token::Operator(Operator::Ne),
                Token::Operator(Operator::Lt),
                Token::Operator(Operator::Gt),
                Token::Operator(Operator::Eq),
                Token::Number("-3.5".into()),
                Token::Number("2.0".into()),
            ]
        );
    }

    #[test]
    fn tokenizes_scalar_operators_without_changing_signed_literals() {
        assert_eq!(
            tokenize("SELECT 1 + -2 * (3 - 4) % 5").unwrap(),
            vec![
                Token::Word("SELECT".into()),
                Token::Number("1".into()),
                Token::Plus,
                Token::Number("-2".into()),
                Token::Star,
                Token::LParen,
                Token::Number("3".into()),
                Token::Minus,
                Token::Number("4".into()),
                Token::RParen,
                Token::Percent,
                Token::Number("5".into()),
            ]
        );
    }

    #[test]
    fn identifier_scanner_keeps_underscores_and_digits_in_words() {
        assert_eq!(
            tokenize("SELECT _key2, c_9 FROM t2").unwrap(),
            vec![
                Token::Word("SELECT".into()),
                Token::Word("_key2".into()),
                Token::Comma,
                Token::Word("c_9".into()),
                Token::Word("FROM".into()),
                Token::Word("t2".into()),
            ]
        );
    }

    #[test]
    fn rejects_unpaired_exclamation_mark() {
        assert!(tokenize("SELECT 1 ! 2").is_err());
        assert_eq!(
            tokenize("1!=2").unwrap(),
            vec![
                Token::Number("1".into()),
                Token::Operator(Operator::Ne),
                Token::Number("2".into()),
            ]
        );
    }

    #[test]
    fn rejects_incomplete_numbers_and_accepts_exact_size_limit() {
        for sql in ["1.", "-.2", "SELECT é"] {
            assert!(tokenize(sql).is_err(), "{sql:?}");
        }
        assert!(tokenize(&" ".repeat(4096)).is_ok());
    }
}
