use crate::{Operator, ParseError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    Word(String),
    Number(String),
    Star,
    Percent,
    Comma,
    LParen,
    RParen,
    Semicolon,
    Operator(Operator),
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
            b'0'..=b'9' | b'-' => {
                let start = index;
                if byte == b'-' {
                    index += 1;
                    if !matches!(bytes.get(index), Some(b'0'..=b'9')) {
                        return Err(ParseError("expected digits after '-'".into()));
                    }
                }
                while matches!(bytes.get(index), Some(b'0'..=b'9')) {
                    index += 1;
                }
                if bytes.get(index) == Some(&b'.') {
                    index += 1;
                    if !matches!(bytes.get(index), Some(b'0'..=b'9')) {
                        return Err(ParseError("expected digits after decimal point".into()));
                    }
                    while matches!(bytes.get(index), Some(b'0'..=b'9')) {
                        index += 1;
                    }
                }
                Token::Number(sql[start..index].into())
            }
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
}
