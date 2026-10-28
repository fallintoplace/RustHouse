mod scanning;

use self::scanning::*;

use crate::{Operator, ParseError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    Word(String),
    Number(String),
    String(String),
    Star,
    Slash,
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
            b'*' | b'/' | b'%' | b'+' | b',' | b'.' | b'(' | b')' | b';' => {
                index += 1;
                punctuation_token(byte)
            }
            b'#' => {
                index += 1;
                while index < bytes.len() && !matches!(bytes[index], b'\r' | b'\n') {
                    index += 1;
                }
                continue;
            }
            b'-' if follows_binary_minus_context(&tokens) => {
                index += 1;
                Token::Minus
            }
            b'-' if bytes.get(index + 1) == Some(&b'.') => {
                return Err(ParseError("expected digits after '-'".into()));
            }
            b'-' if !is_ascii_digit_at(bytes, index + 1) => {
                index += 1;
                Token::Minus
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
    fn digit_classification_handles_token_boundaries() {
        assert!(is_ascii_digit_at(b"9a", 0));
        assert!(!is_ascii_digit_at(b"9a", 1));
        assert!(!is_ascii_digit_at(b"9a", 2));
        assert!(!is_ascii_digit_at("é".as_bytes(), 0));
    }

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
    fn distinguishes_binary_subtraction_from_negative_literals() {
        assert_eq!(
            tokenize("1-2 + -3 * (4)-5").unwrap(),
            vec![
                Token::Number("1".into()),
                Token::Minus,
                Token::Number("2".into()),
                Token::Plus,
                Token::Number("-3".into()),
                Token::Star,
                Token::LParen,
                Token::Number("4".into()),
                Token::RParen,
                Token::Minus,
                Token::Number("5".into()),
            ]
        );
    }

    #[test]
    fn identifies_subtraction_adjacent_to_column_names() {
        assert_eq!(
            tokenize("count-2 count -2 SELECT -3").unwrap(),
            vec![
                Token::Word("count".into()), Token::Minus, Token::Number("2".into()),
                Token::Word("count".into()), Token::Minus, Token::Number("2".into()),
                Token::Word("SELECT".into()), Token::Number("-3".into()),
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
    fn handles_empty_doubled_and_unterminated_string_literals() {
        assert_eq!(
            tokenize("'' 'it''s'").unwrap(),
            vec![Token::String("".into()), Token::String("it's".into())]
        );
        assert!(tokenize("'unterminated").is_err());
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
    #[test]
    fn skips_clickhouse_hash_line_comments() {
        assert_eq!(
            tokenize("SELECT 1 # comment\n + 2").unwrap(),
            vec![
                Token::Word("SELECT".into()),
                Token::Number("1".into()),
                Token::Plus,
                Token::Number("2".into()),
            ]
        );
        assert!(tokenize("# only a comment").unwrap().is_empty());
    }

}
