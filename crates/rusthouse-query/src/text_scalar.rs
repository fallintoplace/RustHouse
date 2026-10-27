use super::{ConstantValue, ParseError, Parser, Token};

const MAX_DEPTH: usize = 16;
const MAX_TEXT_BYTES: usize = 1_048_576;

pub(super) fn is_function(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "LENGTH"
            | "CHAR_LENGTH"
            | "IS_INT64"
            | "BTRIM"
            | "LOWER"
            | "UPPER"
            | "REVERSE"
            | "CONCAT"
            | "CONCAT_WS"
            | "SUBSTRING"
            | "SPLIT_PART"
            | "TRIM"
            | "LPAD"
            | "REPLACE"
            | "STRPOS"
            | "LEFT"
            | "STARTS_WITH"
            | "RIGHT"
            | "ENDS_WITH"
            | "REPEAT"
            | "RPAD"
    )
}

pub(super) fn parse(parser: &mut Parser) -> Result<ConstantValue, ParseError> {
    parse_at_depth(parser, 0)
}

fn parse_at_depth(parser: &mut Parser, depth: usize) -> Result<ConstantValue, ParseError> {
    if depth >= MAX_DEPTH {
        return Err(ParseError("string functions nest at most 16 levels".into()));
    }
    let name = parser.word()?.to_ascii_uppercase();
    require(parser, Token::LParen, "string function requires '('")?;
    let value = match name.as_str() {
        "LENGTH" | "CHAR_LENGTH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "LENGTH requires one argument")?;
            match input {
                Some(value) => {
                    let length = if name == "CHAR_LENGTH" {
                        value.chars().count()
                    } else {
                        value.len()
                    };
                    ConstantValue::Integer(
                        i64::try_from(length)
                            .map_err(|_| ParseError("string length exceeds Int64".into()))?,
                    )
                }
                None => ConstantValue::Null,
            }
        }
        "LOWER" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "LOWER requires one argument")?;
            match input {
                Some(value) => {
                    let result = value.to_lowercase();
                    if result.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("LOWER result is too large".into()));
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "UPPER" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "UPPER requires one argument")?;
            match input {
                Some(value) => {
                    let result = value.to_uppercase();
                    if result.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("UPPER result is too large".into()));
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "IS_INT64" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "IS_INT64 requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(i64::from(value.parse::<i64>().is_ok())),
                None => ConstantValue::Null,
            }
        }
        "REVERSE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "REVERSE requires one argument")?;
            match input {
                Some(value) => ConstantValue::Text(value.chars().rev().collect()),
                None => ConstantValue::Null,
            }
        }
        "CONCAT_WS" => {
            let separator = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "CONCAT_WS requires at least one value")?;
            let mut result = String::new();
            let mut written = false;
            let mut count = 0;
            loop {
                if count == 16 {
                    return Err(ParseError("CONCAT_WS supports at most 16 values".into()));
                }
                let value = text_arg(parser, depth + 1)?;
                if let (Some(separator), Some(value)) = (separator.as_ref(), value) {
                    let separator_len = if written { separator.len() } else { 0 };
                    let total = result
                        .len()
                        .checked_add(separator_len)
                        .and_then(|size| size.checked_add(value.len()))
                        .ok_or_else(|| ParseError("CONCAT_WS result is too large".into()))?;
                    if total > MAX_TEXT_BYTES {
                        return Err(ParseError("CONCAT_WS result is too large".into()));
                    }
                    if written {
                        result.push_str(separator);
                    }
                    result.push_str(&value);
                    written = true;
                }
                count += 1;
                if !parser.take(Token::Comma) {
                    break;
                }
            }
            require(parser, Token::RParen, "expected ')' after CONCAT_WS arguments")?;
            if separator.is_some() {
                ConstantValue::Text(result)
            } else {
                ConstantValue::Null
            }
        }
        "CONCAT" => {
            let mut result = String::new();
            let mut count = 0;
            let mut is_null = false;
            loop {
                if count == 16 {
                    return Err(ParseError("CONCAT supports at most 16 arguments".into()));
                }
                match text_arg(parser, depth + 1)? {
                    Some(value) => {
                        let total = result
                            .len()
                            .checked_add(value.len())
                            .ok_or_else(|| ParseError("CONCAT result is too large".into()))?;
                        if total > MAX_TEXT_BYTES {
                            return Err(ParseError("CONCAT result is too large".into()));
                        }
                        result.push_str(&value);
                    }
                    None => is_null = true,
                }
                count += 1;
                if !parser.take(Token::Comma) {
                    break;
                }
            }
            if count < 2 {
                return Err(ParseError("CONCAT requires at least two arguments".into()));
            }
            require(parser, Token::RParen, "expected ')' after CONCAT arguments")?;
            if is_null {
                ConstantValue::Null
            } else {
                ConstantValue::Text(result)
            }
        }
        "SPLIT_PART" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "SPLIT_PART requires a delimiter")?;
            let delimiter = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "SPLIT_PART requires a field index")?;
            let index = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after SPLIT_PART arguments")?;
            match (input, delimiter, index) {
                (Some(input), Some(delimiter), Some(index)) => {
                    if delimiter.is_empty() {
                        return Err(ParseError("SPLIT_PART delimiter cannot be empty".into()));
                    }
                    if index < 1 {
                        return Err(ParseError("SPLIT_PART index must be positive".into()));
                    }
                    let index = usize::try_from(index - 1).unwrap_or(usize::MAX);
                    ConstantValue::Text(
                        input.split(delimiter.as_str()).nth(index).unwrap_or("").to_owned(),
                    )
                }
                _ => ConstantValue::Null,
            }
        }
        "SUBSTRING" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "SUBSTRING requires a start position")?;
            let position = integer_arg(parser, depth + 1)?;
            let length = if parser.take(Token::Comma) {
                Some(integer_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(
                parser,
                Token::RParen,
                "expected ')' after SUBSTRING arguments",
            )?;
            match (input, position, length) {
                (Some(value), Some(position), length) => {
                    if position < 1 {
                        return Err(ParseError("SUBSTRING positions are 1-based".into()));
                    }
                    let start = usize::try_from(position - 1).unwrap_or(usize::MAX);
                    let count = match length {
                        None => usize::MAX,
                        Some(Some(count)) if count >= 0 => {
                            usize::try_from(count).unwrap_or(usize::MAX)
                        }
                        Some(Some(_)) => {
                            return Err(ParseError("SUBSTRING length must be nonnegative".into()));
                        }
                        Some(None) => return Ok(ConstantValue::Null),
                    };
                    ConstantValue::Text(value.chars().skip(start).take(count).collect())
                }
                _ => ConstantValue::Null,
            }
        }
        "LPAD" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "LPAD requires a width")?;
            let width = integer_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "LPAD requires padding text")?;
            let padding = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after LPAD arguments")?;
            match (input, width, padding) {
                (Some(input), Some(width), Some(padding)) => {
                    let width = usize::try_from(width)
                        .map_err(|_| ParseError("LPAD width must be nonnegative".into()))?;
                    if width > MAX_TEXT_BYTES {
                        return Err(ParseError("LPAD result is too large".into()));
                    }
                    let tail: String = input.chars().take(width).collect();
                    if tail.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("LPAD result is too large".into()));
                    }
                    let missing = width - tail.chars().count();
                    if missing > 0 && padding.is_empty() {
                        return Err(ParseError("LPAD padding cannot be empty".into()));
                    }
                    let mut result = String::new();
                    for character in padding.chars().cycle().take(missing) {
                        if result.len() + character.len_utf8() + tail.len() > MAX_TEXT_BYTES {
                            return Err(ParseError("LPAD result is too large".into()));
                        }
                        result.push(character);
                    }
                    result.push_str(&tail);
                    ConstantValue::Text(result)
                }
                _ => ConstantValue::Null,
            }
        }
        "TRIM" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "TRIM requires one argument")?;
            match input {
                Some(value) => ConstantValue::Text(value.trim_matches(' ').to_owned()),
                None => ConstantValue::Null,
            }
        }
        "STRPOS" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "STRPOS requires a substring")?;
            let needle = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after STRPOS arguments")?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    let index = input
                        .find(needle.as_str())
                        .map(|byte_index| input[..byte_index].chars().count() + 1)
                        .unwrap_or(0);
                    ConstantValue::Integer(
                        i64::try_from(index)
                            .map_err(|_| ParseError("STRPOS result exceeds Int64".into()))?,
                    )
                }
                _ => ConstantValue::Null,
            }
        }
        "BTRIM" => {
            let input = text_arg(parser, depth + 1)?;
            let characters = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(parser, Token::RParen, "expected ')' after BTRIM arguments")?;
            match (input, characters) {
                (Some(input), None) => {
                    ConstantValue::Text(input.trim_matches(' ').to_owned())
                }
                (Some(input), Some(Some(characters))) => ConstantValue::Text(
                    input.trim_matches(|character| characters.contains(character)).to_owned(),
                ),
                _ => ConstantValue::Null,
            }
        }
        "REPLACE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "REPLACE requires a search string")?;
            let needle = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "REPLACE requires a replacement string",
            )?;
            let replacement = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after REPLACE arguments",
            )?;
            match (input, needle, replacement) {
                (Some(input), Some(needle), Some(replacement)) => {
                    if needle.is_empty() {
                        ConstantValue::Text(input)
                    } else {
                        let matches = input.match_indices(&needle).count();
                        let removed = matches * needle.len();
                        let inserted = matches
                            .checked_mul(replacement.len())
                            .ok_or_else(|| ParseError("REPLACE result is too large".into()))?;
                        let length = (input.len() - removed)
                            .checked_add(inserted)
                            .ok_or_else(|| ParseError("REPLACE result is too large".into()))?;
                        if length > MAX_TEXT_BYTES {
                            return Err(ParseError("REPLACE result is too large".into()));
                        }
                        ConstantValue::Text(input.replace(&needle, &replacement))
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "STARTS_WITH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "STARTS_WITH requires a prefix")?;
            let prefix = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after STARTS_WITH arguments")?;
            match (input, prefix) {
                (Some(input), Some(prefix)) => {
                    ConstantValue::Integer(i64::from(input.starts_with(prefix.as_str())))
                }
                _ => ConstantValue::Null,
            }
        }
        "LEFT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "LEFT requires a character count")?;
            let count = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after LEFT arguments")?;
            match (input, count) {
                (Some(value), Some(count)) => {
                    if count < 0 {
                        return Err(ParseError("LEFT count must be nonnegative".into()));
                    }
                    let count = usize::try_from(count).unwrap_or(usize::MAX);
                    ConstantValue::Text(value.chars().take(count).collect())
                }
                _ => ConstantValue::Null,
            }
        }
        "ENDS_WITH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "ENDS_WITH requires a suffix")?;
            let suffix = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after ENDS_WITH arguments")?;
            match (input, suffix) {
                (Some(input), Some(suffix)) => {
                    ConstantValue::Integer(i64::from(input.ends_with(suffix.as_str())))
                }
                _ => ConstantValue::Null,
            }
        }
        "RIGHT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "RIGHT requires a character count")?;
            let count = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after RIGHT arguments")?;
            match (input, count) {
                (Some(value), Some(count)) => {
                    if count < 0 {
                        return Err(ParseError("RIGHT count must be nonnegative".into()));
                    }
                    let count = usize::try_from(count).unwrap_or(usize::MAX);
                    let skip = value.chars().count().saturating_sub(count);
                    ConstantValue::Text(value.chars().skip(skip).collect())
                }
                _ => ConstantValue::Null,
            }
        }
        "RPAD" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "RPAD requires a width")?;
            let width = integer_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "RPAD requires padding text")?;
            let padding = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after RPAD arguments")?;
            match (input, width, padding) {
                (Some(input), Some(width), Some(padding)) => {
                    let width = usize::try_from(width)
                        .map_err(|_| ParseError("RPAD width must be nonnegative".into()))?;
                    if width > MAX_TEXT_BYTES {
                        return Err(ParseError("RPAD result is too large".into()));
                    }
                    let mut result: String = input.chars().take(width).collect();
                    if result.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("RPAD result is too large".into()));
                    }
                    let missing = width - result.chars().count();
                    if missing > 0 && padding.is_empty() {
                        return Err(ParseError("RPAD padding cannot be empty".into()));
                    }
                    for character in padding.chars().cycle().take(missing) {
                        if result.len() + character.len_utf8() > MAX_TEXT_BYTES {
                            return Err(ParseError("RPAD result is too large".into()));
                        }
                        result.push(character);
                    }
                    ConstantValue::Text(result)
                }
                _ => ConstantValue::Null,
            }
        }
        "REPEAT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "REPEAT requires a count")?;
            let count = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after REPEAT arguments")?;
            match (input, count) {
                (Some(value), Some(count)) => {
                    if count < 0 {
                        return Err(ParseError("REPEAT count must be nonnegative".into()));
                    }
                    if value.is_empty() {
                        ConstantValue::Text(String::new())
                    } else {
                        let count = usize::try_from(count).unwrap_or(usize::MAX);
                        let length = value
                            .len()
                            .checked_mul(count)
                            .ok_or_else(|| ParseError("REPEAT result is too large".into()))?;
                        if length > MAX_TEXT_BYTES {
                            return Err(ParseError("REPEAT result is too large".into()));
                        }
                        ConstantValue::Text(value.repeat(count))
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        _ => return Err(ParseError(format!("unsupported string function: {name}"))),
    };
    Ok(value)
}

fn require(parser: &mut Parser, token: Token, message: &str) -> Result<(), ParseError> {
    if parser.take(token) {
        Ok(())
    } else {
        Err(ParseError(message.into()))
    }
}

fn integer_arg(parser: &mut Parser, depth: usize) -> Result<Option<i64>, ParseError> {
    if matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL")
    ) {
        parser.index += 1;
        return Ok(None);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if is_function(word)) {
        return match parse_at_depth(parser, depth)? {
            ConstantValue::Integer(value) => Ok(Some(value)),
            ConstantValue::Null => Ok(None),
            ConstantValue::Text(_) => Err(ParseError("expected an integer expression".into())),
        };
    }
    Ok(Some(super::scalar::parse(parser)?))
}

fn text_arg(parser: &mut Parser, depth: usize) -> Result<Option<String>, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::String(value)) => {
            let result = value.clone();
            parser.index += 1;
            Ok(Some(result))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") => {
            parser.index += 1;
            Ok(None)
        }
        Some(Token::Word(word)) if is_function(word) => match parse_at_depth(parser, depth)? {
            ConstantValue::Text(value) => Ok(Some(value)),
            ConstantValue::Null => Ok(None),
            ConstantValue::Integer(_) => Err(ParseError("expected a string expression".into())),
        },
        _ => Err(ParseError(
            "expected a string literal or string function".into(),
        )),
    }
}
