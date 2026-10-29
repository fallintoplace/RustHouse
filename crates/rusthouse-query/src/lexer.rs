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
    tokenize_with_limit(sql, 64 * 1024)
}

pub(super) fn tokenize_insert(sql: &str) -> Result<Vec<Token>, ParseError> {
    tokenize_with_limit(sql, 1024 * 1024)
}

fn tokenize_with_limit(sql: &str, max_bytes: usize) -> Result<Vec<Token>, ParseError> {
    if sql.len() > max_bytes {
        return Err(ParseError(format!(
            "statement exceeds the {max_bytes}-byte limit"
        )));
    }
    // A one-megabyte INSERT full of one-byte punctuation can otherwise
    // allocate a million 24-byte enum tokens before parsing rejects it.
    let max_tokens = if max_bytes > 64 * 1024 { 262_144 } else { 32_768 };
    let bytes = sql.as_bytes();
    // Avoid repeated vector growth on common SELECT statements and large INSERTs.
    let mut tokens = Vec::with_capacity((sql.len() / 8).min(2048));
    let mut index = if sql.starts_with('\u{feff}') { 3 } else { 0 };
    while index < bytes.len() {
        let byte = bytes[index];
        let token = match byte {
            b' ' | b'\t' | b'\r' | b'\n' | b'\x0b' | b'\x0c' => {
                index += 1;
                continue;
            }
            b'\'' => scan_string(sql, &mut index)?,
            b'.' if is_ascii_digit_at(bytes, index + 1) => scan_number(sql, &mut index)?,
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                if index + 1 >= bytes.len() {
                    return Err(ParseError("unterminated block comment".into()));
                }
                index += 2;
                continue;
            }
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
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
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
                if is_ascii_digit_at(bytes, index + 2) {
                    scan_number(sql, &mut index)?
                } else {
                    return Err(ParseError("expected digits after '-'".into()));
                }
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
        if tokens.len() >= max_tokens {
            return Err(ParseError(format!(
                "statement exceeds the {max_tokens}-token limit"
            )));
        }
        tokens.push(token);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_limits_bound_adversarial_punctuation_statements() {
        assert_eq!(tokenize(&",".repeat(32_768)).unwrap().len(), 32_768);
        assert!(tokenize(&",".repeat(32_769)).is_err());
        assert_eq!(
            tokenize_insert(&",".repeat(262_144)).unwrap().len(),
            262_144
        );
        assert!(tokenize_insert(&",".repeat(262_145)).is_err());
    }

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
        assert!(tokenize(&" ".repeat(64 * 1024)).is_ok());
        assert!(tokenize(&" ".repeat(64 * 1024 + 1)).is_err());
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
                Token::Word("count".into()),
                Token::Minus,
                Token::Number("2".into()),
                Token::Word("count".into()),
                Token::Minus,
                Token::Number("2".into()),
                Token::Word("SELECT".into()),
                Token::Number("-3".into()),
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
        for sql in ["1.", "-.", "SELECT é"] {
            assert!(tokenize(sql).is_err(), "{sql:?}");
        }
        assert!(tokenize(&" ".repeat(4096)).is_ok());
    }
    #[test]
    fn accepts_leading_dot_decimals_and_preserves_subtraction() {
        assert_eq!(
            tokenize(".5, -.25 2-.75").unwrap(),
            vec![
                Token::Number(".5".into()),
                Token::Comma,
                Token::Number("-.25".into()),
                Token::Number("2".into()),
                Token::Minus,
                Token::Number(".75".into()),
            ]
        );
        assert!(tokenize("-.").is_err());
    }

    #[test]
    fn preserves_utf8_strings_around_escaped_delimiters() {
        assert_eq!(
            tokenize(r"SELECT 'café''s', '東京\_b', '🙂'").unwrap(),
            vec![
                Token::Word("SELECT".into()),
                Token::String("café's".into()),
                Token::Comma,
                Token::String(r"東京\_b".into()),
                Token::Comma,
                Token::String("🙂".into()),
            ]
        );
    }

    #[test]
    fn ignores_only_a_leading_utf8_bom() {
        assert_eq!(
            tokenize("\u{feff}SELECT 1").unwrap(),
            vec![Token::Word("SELECT".into()), Token::Number("1".into())]
        );
        assert!(tokenize("SELECT \u{feff}1").is_err());
    }

    #[test]
    fn accepts_ascii_vertical_tab_and_form_feed_separators() {
        assert_eq!(
            tokenize("SELECT\x0b1\x0c+\x0b2").unwrap(),
            vec![
                Token::Word("SELECT".into()),
                Token::Number("1".into()),
                Token::Plus,
                Token::Number("2".into()),
            ]
        );
    }

    #[test]
    fn skips_block_comments_and_rejects_unterminated_blocks() {
        assert_eq!(
            tokenize("SELECT/* hidden */1").unwrap(),
            vec![Token::Word("SELECT".into()), Token::Number("1".into())]
        );
        assert!(tokenize("/* standalone */").unwrap().is_empty());
        assert!(tokenize("SELECT /* never closed").is_err());
        assert!(tokenize("/*").is_err());
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

    #[test]
    fn skips_double_dash_line_comments() {
        assert_eq!(
            tokenize("SELECT 1-- ignore the rest\n + 2").unwrap(),
            vec![
                Token::Word("SELECT".into()),
                Token::Number("1".into()),
                Token::Plus,
                Token::Number("2".into()),
            ]
        );
        assert!(tokenize("-- only a comment").unwrap().is_empty());
    }

    #[test]
    fn tokenizes_large_plain_sql_statements_without_losing_positions() {
        let sql = format!(
            "SELECT {}",
            (0..100)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let tokens = tokenize(&sql).unwrap();
        assert!(matches!(tokens.first(), Some(Token::Word(word)) if word == "SELECT"));
        assert_eq!(tokens.len(), 200);
        assert_eq!(tokens.last(), Some(&Token::Number("99".into())));
    }
}
