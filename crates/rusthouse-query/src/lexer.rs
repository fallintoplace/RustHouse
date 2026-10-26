use crate::{Operator, ParseError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    Word(String),
    Number(String),
    String(String),
    Star,
    Percent,
    Comma,
    LParen,
    RParen,
    Semicolon,
    Operator(Operator),
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
    while matches!(bytes.get(*index), Some(b'0'..=b'9')) {
        *index += 1;
    }
    if bytes.get(*index) == Some(&b'.') {
        *index += 1;
        if !matches!(bytes.get(*index), Some(b'0'..=b'9')) {
            return Err(ParseError("expected digits after decimal point".into()));
        }
        while matches!(bytes.get(*index), Some(b'0'..=b'9')) {
            *index += 1;
        }
    }
    Ok(Token::Number(sql[start..*index].into()))
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
            b'\'' => {
                index += 1;
                let mut value = String::new();
                let mut start = index;
                let mut closed = false;
                while index < bytes.len() {
                    if bytes[index] == b'\'' {
                        value.push_str(&sql[start..index]);
                        if bytes.get(index + 1) == Some(&b'\'') {
                            value.push('\'');
                            index += 2;
                            start = index;
                        } else {
                            index += 1;
                            closed = true;
                            break;
                        }
                    } else {
                        index += 1;
                    }
                }
                if !closed {
                    return Err(ParseError("unterminated string literal".into()));
                }
                Token::String(value)
            }
            b'*' => {
                index += 1;
                Token::Star
            }
            b'%' => {
                index += 1;
                Token::Percent
            }
            b',' => {
                index += 1;
                Token::Comma
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
            b'=' => {
                index += 1;
                Token::Operator(Operator::Eq)
            }
            b'<' => {
                index += 1;
                let op = match bytes.get(index) {
                    Some(b'=') => Some(Operator::Le),
                    Some(b'>') => Some(Operator::Ne),
                    _ => None,
                };
                if let Some(op) = op {
                    index += 1;
                    Token::Operator(op)
                } else {
                    Token::Operator(Operator::Lt)
                }
            }
            b'>' => {
                index += 1;
                if bytes.get(index) == Some(&b'=') {
                    index += 1;
                    Token::Operator(Operator::Ge)
                } else {
                    Token::Operator(Operator::Gt)
                }
            }
            b'!' if bytes.get(index + 1) == Some(&b'=') => {
                index += 2;
                Token::Operator(Operator::Ne)
            }
            b'0'..=b'9' | b'-' => scan_number(sql, &mut index)?,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = index;
                index += 1;
                while matches!(
                    bytes.get(index),
                    Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
                ) {
                    index += 1;
                }
                Token::Word(sql[start..index].into())
            }
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
    fn rejects_incomplete_numbers_and_accepts_exact_size_limit() {
        for sql in ["-", "1.", "-.2", "SELECT é"] {
            assert!(tokenize(sql).is_err(), "{sql:?}");
        }
        assert!(tokenize(&" ".repeat(4096)).is_ok());
    }
}
