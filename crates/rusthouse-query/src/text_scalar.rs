use super::{ConstantValue, ParseError, Parser, Token};

const MAX_DEPTH: usize = 16;
const MAX_TEXT_BYTES: usize = 1_048_576;

pub(super) fn is_function(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "LENGTH"
            | "LENGTHUTF8"
            | "CHAR_LENGTH"
            | "OCTET_LENGTH"
            | "ASCII"
            | "UTF16_LENGTH"
            | "HASSUBSEQUENCE"
            | "CHR"
            | "STRINGBYTESUNIQ"
            | "BIT_LENGTH"
            | "IS_INT64"
            | "BTRIM"
            | "TRIMBOTH"
            | "RTRIM"
            | "CUTQUERYSTRING"
            | "COLLAPSE_WHITESPACE"
            | "LOWER"
            | "HEX_ENCODE"
            | "HEX"
            | "IS_ASCII"
            | "UPPER"
            | "URL_DECODE"
            | "IS_EMPTY"
            | "EMPTY"
            | "REVERSE"
            | "REVERSEUTF8"
            | "INITCAP"
            | "CONCAT"
            | "QUOTE_LITERAL"
            | "CUTQUERYSTRINGANDFRAGMENT"
            | "COALESCE"
            | "CONCAT_WS"
            | "CUTFRAGMENT"
            | "URL_ENCODE"
            | "FRAGMENT"
            | "JSON_QUOTE"
            | "ISVALIDJSON"
            | "JSONHAS"
            | "JSONEXTRACTSTRING"
            | "JSONEXTRACTRAW"
            | "SUBSTRING"
            | "YYYYMMDDTODATE"
            | "HASSUBSEQUENCECASEINSENSITIVE"
            | "OVERLAY"
            | "QUERYSTRING"
            | "SPLIT_PART"
            | "SUBSTRING_INDEX"
            | "SUBSTRINGINDEXUTF8"
            | "QUERYSTRINGANDFRAGMENT"
            | "FORMATREADABLETIMEDELTA"
            | "TO_INT64"
            | "TRIM"
            | "LTRIM"
            | "TRIMLEFT"
            | "BASENAME"
            | "TRIM_WHITESPACE"
            | "TRY_TO_INT64"
            | "TOINT64ORNULL"
            | "LPAD"
            | "LEFTPADUTF8"
            | "COUNTSUBSTRINGSCASEINSENSITIVE"
            | "HEX_DECODE"
            | "REPLACE"
            | "REPLACEALL"
            | "PROTOCOL"
            | "TRANSLATE"
            | "POSITIONCASEINSENSITIVE"
            | "STRPOS"
            | "POSITIONUTF8"
            | "COUNT_SUBSTRING"
            | "COUNTSUBSTRINGS"
            | "EXTRACTURLPARAMETER"
            | "NULLIF"
            | "LAST_INDEX_OF"
            | "PATH"
            | "LEFT"
            | "REPLACE_FIRST"
            | "REPLACEONE"
            | "STARTS_WITH"
            | "STARTSWITH"
            | "HASTOKEN"
            | "CONTAINS"
            | "STRIP_PREFIX"
            | "RIGHT"
            | "BASE64_ENCODE"
            | "BASE64ENCODE"
            | "HASTOKENCASEINSENSITIVE"
            | "ENDS_WITH"
            | "ENDSWITH"
            | "PATHFULL"
            | "REPLACE_LAST"
            | "NETLOC"
            | "STRIP_SUFFIX"
            | "REPEAT"
            | "BASE64_DECODE"
            | "BASE64DECODE"
            | "APPENDTRAILINGCHARIFABSENT"
            | "RPAD"
            | "LOWERUTF8"
            | "SPACE"
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

        "LENGTHUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "lengthUTF8 requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(
                    i64::try_from(value.chars().count())
                        .map_err(|_| ParseError("lengthUTF8 result exceeds Int64".into()))?,
                ),
                None => ConstantValue::Null,
            }
        }
        "FORMATREADABLETIMEDELTA" => {
            let value = integer_arg(parser, depth + 1)?;
            let maximum_unit = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            let minimum_unit = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(parser, Token::RParen, "formatReadableTimeDelta requires ')'")?;
            if value.is_none() || matches!(maximum_unit, Some(None))
                || matches!(minimum_unit, Some(None))
            {
                ConstantValue::Null
            } else {
                let maximum = maximum_unit.flatten().unwrap_or_else(|| "years".into());
                let minimum = minimum_unit.flatten().unwrap_or_else(|| "seconds".into());
                const UNITS: &[(&str, u64)] = &[
                    ("years", 31_536_000), ("months", 2_635_200),
                    ("days", 86_400), ("hours", 3_600),
                    ("minutes", 60), ("seconds", 1),
                ];
                let max_index = UNITS.iter().position(|(name, _)| *name == maximum)
                    .ok_or_else(|| ParseError("invalid maximum time unit".into()))?;
                let min_index = UNITS.iter().position(|(name, _)| *name == minimum)
                    .ok_or_else(|| ParseError("invalid minimum time unit".into()))?;
                if max_index > min_index {
                    return Err(ParseError("minimum unit exceeds maximum unit".into()));
                }
                let seconds = value.expect("validated nonnull time value");
                let mut remaining = seconds.unsigned_abs();
                let mut parts = Vec::new();
                for (index, &(name, unit_seconds)) in UNITS.iter().enumerate()
                    .skip(max_index).take(min_index - max_index + 1)
                {
                    let count = remaining / unit_seconds;
                    remaining %= unit_seconds;
                    if count > 0 || (index == min_index && parts.is_empty()) {
                        let singular = name.trim_end_matches('s');
                        parts.push(format!("{count} {singular}{}", if count == 1 { "" } else { "s" }));
                    }
                }
                let mut formatted = if parts.len() <= 1 {
                    parts.join("")
                } else {
                    let last = parts.pop().expect("at least two time parts");
                    format!("{} and {last}", parts.join(", "))
                };
                if seconds < 0 {
                    formatted.insert(0, '-');
                }
                ConstantValue::Text(formatted)
            }
        }
        "ASCII" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "ASCII requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(
                    value
                        .chars()
                        .next()
                        .map(|ch| i64::from(ch as u32))
                        .unwrap_or(0),
                ),
                None => ConstantValue::Null,
            }
        }
        "LENGTH" | "CHAR_LENGTH" | "OCTET_LENGTH" => {
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
        "CUTQUERYSTRING" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "cutQueryString requires one argument",
            )?;
            match input {
                Some(input) => {
                    let (before_fragment, fragment) = match input.split_once('#') {
                        Some((head, tail)) => (head, Some(tail)),
                        None => (input.as_str(), None),
                    };
                    let prefix = before_fragment
                        .split_once('?')
                        .map_or(before_fragment, |(head, _)| head);
                    let mut output = prefix.to_owned();
                    if let Some(fragment) = fragment {
                        output.push('#');
                        output.push_str(fragment);
                    }
                    ConstantValue::Text(output)
                }
                None => ConstantValue::Null,
            }
        }
        "HEX_ENCODE" | "HEX" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "HEX_ENCODE requires one argument")?;
            match input {
                Some(value) => {
                    let length = value
                        .len()
                        .checked_mul(2)
                        .ok_or_else(|| ParseError("HEX_ENCODE result is too large".into()))?;
                    if length > MAX_TEXT_BYTES {
                        return Err(ParseError("HEX_ENCODE result is too large".into()));
                    }
                    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
                    let mut output = String::with_capacity(length);
                    for byte in value.bytes() {
                        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
                        output.push(char::from(DIGITS[usize::from(byte & 15)]));
                    }
                    ConstantValue::Text(output)
                }
                None => ConstantValue::Null,
            }
        }
        "UTF16_LENGTH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "UTF16_LENGTH requires one argument")?;
            match input {
                Some(value) => {
                    let length = i64::try_from(value.encode_utf16().count())
                        .map_err(|_| ParseError("UTF16_LENGTH result exceeds Int64".into()))?;
                    ConstantValue::Integer(length)
                }
                None => ConstantValue::Null,
            }
        }
        "STRINGBYTESUNIQ" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "stringBytesUniq requires one argument",
            )?;
            match input {
                Some(value) => {
                    let mut seen = [false; 256];
                    for byte in value.bytes() {
                        seen[usize::from(byte)] = true;
                    }
                    ConstantValue::Integer(seen.iter().filter(|&&present| present).count() as i64)
                }
                None => ConstantValue::Null,
            }
        }
        "BIT_LENGTH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "BIT_LENGTH requires one argument")?;
            match input {
                Some(value) => {
                    let bits = value
                        .len()
                        .checked_mul(8)
                        .and_then(|length| i64::try_from(length).ok())
                        .ok_or_else(|| ParseError("BIT_LENGTH result exceeds Int64".into()))?;
                    ConstantValue::Integer(bits)
                }
                None => ConstantValue::Null,
            }
        }
        "LOWER" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "LOWER requires one argument")?;
            match input {
                Some(value) => {
                    let result = value.to_ascii_lowercase();
                    if result.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("LOWER result is too large".into()));
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "HASSUBSEQUENCE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "hasSubsequence requires a needle")?;
            let needle = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after hasSubsequence")?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    let mut required = needle.bytes();
                    let mut next = required.next();
                    for byte in input.bytes() {
                        if next == Some(byte) {
                            next = required.next();
                        }
                    }
                    ConstantValue::Integer(i64::from(next.is_none()))
                }
                _ => ConstantValue::Null,
            }
        }
        "CHR" => {
            let input = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "CHR requires one argument")?;
            match input {
                Some(value) => {
                    let character = u32::try_from(value)
                        .ok()
                        .filter(|code| *code != 0)
                        .and_then(char::from_u32)
                        .ok_or_else(|| {
                            ParseError("CHR requires a nonzero Unicode scalar".into())
                        })?;
                    ConstantValue::Text(character.to_string())
                }
                None => ConstantValue::Null,
            }
        }
        "UPPER" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "UPPER requires one argument")?;
            match input {
                Some(value) => {
                    let result = value.to_ascii_uppercase();
                    if result.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("UPPER result is too large".into()));
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "INITCAP" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "INITCAP requires one argument")?;
            match input {
                Some(value) => {
                    let mut result = String::with_capacity(value.len());
                    let mut word_start = true;
                    for ch in value.chars() {
                        let transformed = if word_start {
                            ch.to_uppercase().to_string()
                        } else {
                            ch.to_lowercase().to_string()
                        };
                        if result.len() + transformed.len() > MAX_TEXT_BYTES {
                            return Err(ParseError("INITCAP result is too large".into()));
                        }
                        result.push_str(&transformed);
                        word_start = !ch.is_alphanumeric();
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "IS_ASCII" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "IS_ASCII requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(i64::from(value.is_ascii())),
                None => ConstantValue::Null,
            }
        }
        "IS_EMPTY" | "EMPTY" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "IS_EMPTY requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(i64::from(value.is_empty())),
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
        "REVERSE" | "REVERSEUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "REVERSE requires one argument")?;
            match input {
                Some(value) => ConstantValue::Text(value.chars().rev().collect()),
                None => ConstantValue::Null,
            }
        }
        "CUTFRAGMENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "cutFragment requires one argument")?;
            match input {
                Some(input) => ConstantValue::Text(
                    input
                        .split_once('#')
                        .map_or(input.as_str(), |(prefix, _)| prefix)
                        .to_owned(),
                ),
                None => ConstantValue::Null,
            }
        }
        "URL_ENCODE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "URL_ENCODE requires one argument")?;
            match input {
                Some(input) => {
                    fn unreserved(byte: u8) -> bool {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
                    }
                    let mut length = 0usize;
                    for byte in input.bytes() {
                        length = length
                            .checked_add(if unreserved(byte) { 1 } else { 3 })
                            .ok_or_else(|| ParseError("URL_ENCODE result is too large".into()))?;
                        if length > MAX_TEXT_BYTES {
                            return Err(ParseError("URL_ENCODE result is too large".into()));
                        }
                    }
                    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
                    let mut result = String::with_capacity(length);
                    for byte in input.bytes() {
                        if unreserved(byte) {
                            result.push(char::from(byte));
                        } else {
                            result.push('%');
                            result.push(char::from(DIGITS[usize::from(byte >> 4)]));
                            result.push(char::from(DIGITS[usize::from(byte & 15)]));
                        }
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "CUTQUERYSTRINGANDFRAGMENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "cutQueryStringAndFragment requires one argument",
            )?;
            match input {
                Some(input) => {
                    let end = input.find(['?', '#']).unwrap_or(input.len());
                    ConstantValue::Text(input[..end].to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "COALESCE" => {
            let mut first = None;
            let mut count = 0;
            loop {
                if count == 16 {
                    return Err(ParseError("COALESCE supports at most 16 arguments".into()));
                }
                let value = text_arg(parser, depth + 1)?;
                if first.is_none() {
                    first = value;
                }
                count += 1;
                if !parser.take(Token::Comma) {
                    break;
                }
            }
            require(
                parser,
                Token::RParen,
                "expected ')' after COALESCE arguments",
            )?;
            first.map_or(ConstantValue::Null, ConstantValue::Text)
        }
        "CONCAT_WS" => {
            let separator = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "CONCAT_WS requires at least one value",
            )?;
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
            require(
                parser,
                Token::RParen,
                "expected ')' after CONCAT_WS arguments",
            )?;
            if separator.is_some() {
                ConstantValue::Text(result)
            } else {
                ConstantValue::Null
            }
        }
        "QUOTE_LITERAL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "QUOTE_LITERAL requires one argument")?;
            match input {
                Some(value) => {
                    let quotes = value.bytes().filter(|byte| *byte == b'\'').count();
                    let length = value
                        .len()
                        .checked_add(quotes)
                        .and_then(|n| n.checked_add(2))
                        .ok_or_else(|| ParseError("QUOTE_LITERAL result is too large".into()))?;
                    if length > MAX_TEXT_BYTES {
                        return Err(ParseError("QUOTE_LITERAL result is too large".into()));
                    }
                    let mut output = String::with_capacity(length);
                    output.push('\'');
                    for ch in value.chars() {
                        if ch == '\'' {
                            output.push('\'');
                        }
                        output.push(ch);
                    }
                    output.push('\'');
                    ConstantValue::Text(output)
                }
                None => ConstantValue::Null,
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
        "QUERYSTRING" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "queryString requires one argument")?;
            match input {
                Some(input) => {
                    let before_fragment = input
                        .split_once('#')
                        .map_or(input.as_str(), |(head, _)| head);
                    let query = before_fragment
                        .split_once('?')
                        .map_or("", |(_, query)| query);
                    ConstantValue::Text(query.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "SUBSTRING_INDEX" | "SUBSTRINGINDEXUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "SUBSTRING_INDEX requires a delimiter")?;
            let delimiter = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "SUBSTRING_INDEX requires a field count",
            )?;
            let count = integer_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after SUBSTRING_INDEX arguments",
            )?;
            match (input, delimiter, count) {
                (Some(input), Some(delimiter), Some(count)) => {
                    if delimiter.is_empty() || count == 0 {
                        ConstantValue::Text(String::new())
                    } else {
                        let field = usize::try_from(count.unsigned_abs())
                            .unwrap_or(usize::MAX)
                            .saturating_sub(1);
                        let result = if count > 0 {
                            input
                                .match_indices(delimiter.as_str())
                                .nth(field)
                                .map(|(index, _)| &input[..index])
                                .unwrap_or(&input)
                        } else {
                            input
                                .rmatch_indices(delimiter.as_str())
                                .nth(field)
                                .map(|(index, _)| &input[index + delimiter.len()..])
                                .unwrap_or(&input)
                        };
                        ConstantValue::Text(result.to_owned())
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "QUERYSTRINGANDFRAGMENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "queryStringAndFragment requires one argument",
            )?;
            match input {
                Some(input) => {
                    let before_fragment = input
                        .split_once('#')
                        .map_or(input.as_str(), |(head, _)| head);
                    let query = before_fragment
                        .find('?')
                        .map_or("", |position| &input[position + 1..]);
                    ConstantValue::Text(query.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "SPLIT_PART" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "SPLIT_PART requires a delimiter")?;
            let delimiter = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "SPLIT_PART requires a field index")?;
            let index = integer_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after SPLIT_PART arguments",
            )?;
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
                        input
                            .split(delimiter.as_str())
                            .nth(index)
                            .unwrap_or("")
                            .to_owned(),
                    )
                }
                _ => ConstantValue::Null,
            }
        }
        "HASSUBSEQUENCECASEINSENSITIVE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "hasSubsequenceCaseInsensitive requires a needle",
            )?;
            let needle = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after hasSubsequenceCaseInsensitive",
            )?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    let mut required = needle.bytes().map(|byte| byte.to_ascii_lowercase());
                    let mut next = required.next();
                    for byte in input.bytes() {
                        if next == Some(byte.to_ascii_lowercase()) {
                            next = required.next();
                        }
                    }
                    ConstantValue::Integer(i64::from(next.is_none()))
                }
                _ => ConstantValue::Null,
            }
        }
        "OVERLAY" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "OVERLAY requires replacement text")?;
            let replacement = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "OVERLAY requires a start position")?;
            let start = integer_arg(parser, depth + 1)?;
            let length = if parser.take(Token::Comma) {
                Some(integer_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(
                parser,
                Token::RParen,
                "expected ')' after OVERLAY arguments",
            )?;
            match (input, replacement, start) {
                (Some(input), Some(replacement), Some(start)) => {
                    if start < 1 {
                        return Err(ParseError("OVERLAY positions are 1-based".into()));
                    }
                    let remove = match length {
                        None => replacement.chars().count(),
                        Some(Some(length)) if length >= 0 => {
                            usize::try_from(length).unwrap_or(usize::MAX)
                        }
                        Some(Some(_)) => {
                            return Err(ParseError("OVERLAY length must be nonnegative".into()));
                        }
                        Some(None) => return Ok(ConstantValue::Null),
                    };
                    let start = usize::try_from(start - 1).unwrap_or(usize::MAX);
                    let prefix = input
                        .char_indices()
                        .nth(start)
                        .map_or(input.len(), |(index, _)| index);
                    let suffix = input
                        .char_indices()
                        .nth(start.saturating_add(remove))
                        .map_or(input.len(), |(index, _)| index);
                    let size = prefix
                        .checked_add(replacement.len())
                        .and_then(|size| size.checked_add(input.len() - suffix))
                        .ok_or_else(|| ParseError("OVERLAY result is too large".into()))?;
                    if size > MAX_TEXT_BYTES {
                        return Err(ParseError("OVERLAY result is too large".into()));
                    }
                    let mut result = String::with_capacity(size);
                    result.push_str(&input[..prefix]);
                    result.push_str(&replacement);
                    result.push_str(&input[suffix..]);
                    ConstantValue::Text(result)
                }
                _ => ConstantValue::Null,
            }
        }
        "FRAGMENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "fragment requires one argument")?;
            match input {
                Some(input) => ConstantValue::Text(
                    input
                        .split_once('#')
                        .map_or("", |(_, fragment)| fragment)
                        .to_owned(),
                ),
                None => ConstantValue::Null,
            }
        }
        "YYYYMMDDTODATE" => {
            let input = integer_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "YYYYMMDDToDate requires one argument",
            )?;
            match input {
                Some(value) => {
                    let digits = value.to_string();
                    if digits.len() != 8 || !digits.is_ascii() {
                        return Err(ParseError("YYYYMMDDToDate requires a 8-digit integer".into()));
                    }
                    let timestamp = format!(
                        "{}-{}-{}",
                        &digits[..4],
                        &digits[4..6],
                        &digits[6..8],
                    );
                    if rusthouse_storage::parse_date(&timestamp).is_none() {
                        return Err(ParseError("YYYYMMDDToDate requires a valid UTC calendar value".into()));
                    }
                    ConstantValue::Text(timestamp)
                }
                None => ConstantValue::Null,
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
        "HEX_DECODE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "HEX_DECODE requires one argument")?;
            match input {
                Some(value) => {
                    if value.len() % 2 != 0 || !value.is_ascii() {
                        return Err(ParseError(
                            "HEX_DECODE requires even-length ASCII hex".into(),
                        ));
                    }
                    let mut bytes = Vec::with_capacity(value.len() / 2);
                    for pair in value.as_bytes().chunks_exact(2) {
                        let hex = std::str::from_utf8(pair)
                            .map_err(|_| ParseError("invalid HEX_DECODE input".into()))?;
                        let byte = u8::from_str_radix(hex, 16)
                            .map_err(|_| ParseError("invalid HEX_DECODE input".into()))?;
                        bytes.push(byte);
                    }
                    let decoded = String::from_utf8(bytes)
                        .map_err(|_| ParseError("HEX_DECODE result is not UTF-8".into()))?;
                    ConstantValue::Text(decoded)
                }
                None => ConstantValue::Null,
            }
        }
        "TO_INT64" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "TO_INT64 requires one argument")?;
            match input {
                Some(value) => ConstantValue::Integer(
                    value
                        .parse::<i64>()
                        .map_err(|_| ParseError("TO_INT64 requires an Int64 string".into()))?,
                ),
                None => ConstantValue::Null,
            }
        }
        "LPAD" | "LEFTPADUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "LPAD requires a width")?;
            let width = integer_arg(parser, depth + 1)?;
            let padding = if parser.take(Token::Comma) {
                text_arg(parser, depth + 1)?
            } else {
                Some(" ".to_owned())
            };
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
        "PROTOCOL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "protocol requires one argument")?;
            match input {
                Some(input) => {
                    let scheme = input.split_once(':').map_or("", |(scheme, _)| scheme);
                    let valid = scheme
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphabetic)
                        && scheme.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
                        });
                    ConstantValue::Text(if valid { scheme } else { "" }.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "LTRIM" | "TRIMLEFT" => {
            let input = text_arg(parser, depth + 1)?;
            let characters = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(parser, Token::RParen, "expected ')' after LTRIM arguments")?;
            match (input, characters) {
                (Some(input), None) => {
                    ConstantValue::Text(input.trim_start_matches(' ').to_owned())
                }
                (Some(input), Some(Some(characters))) => ConstantValue::Text(
                    input
                        .trim_start_matches(|ch| characters.contains(ch))
                        .to_owned(),
                ),
                _ => ConstantValue::Null,
            }
        }
        "BASENAME" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "basename requires one argument")?;
            match input {
                Some(input) => {
                    ConstantValue::Text(input.rsplit(['/', '\\']).next().unwrap_or("").to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "TRIM_WHITESPACE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "TRIM_WHITESPACE requires one argument",
            )?;
            match input {
                Some(value) => ConstantValue::Text(value.trim().to_owned()),
                None => ConstantValue::Null,
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
        "COUNTSUBSTRINGSCASEINSENSITIVE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "countSubstringsCaseInsensitive requires a needle",
            )?;
            let needle = text_arg(parser, depth + 1)?;
            let start = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(1)
            };
            require(
                parser,
                Token::RParen,
                "expected ')' after countSubstringsCaseInsensitive",
            )?;
            match (input, needle, start) {
                (Some(input), Some(needle), Some(start)) => {
                    if start < 0 {
                        return Err(ParseError(
                            "countSubstringsCaseInsensitive start must be nonnegative".into(),
                        ));
                    }
                    let offset = usize::try_from(start.max(1) - 1).unwrap_or(usize::MAX);
                    let bytes = input.as_bytes().get(offset..).unwrap_or_default();
                    let needle = needle.as_bytes();
                    let mut count = 0_i64;
                    if !needle.is_empty() && needle.len() <= bytes.len() {
                        let mut cursor = 0;
                        while cursor <= bytes.len() - needle.len() {
                            if bytes[cursor..cursor + needle.len()].eq_ignore_ascii_case(needle) {
                                count += 1;
                                cursor += needle.len();
                            } else {
                                cursor += 1;
                            }
                        }
                    }
                    ConstantValue::Integer(count)
                }
                _ => ConstantValue::Null,
            }
        }
        "COUNT_SUBSTRING" | "COUNTSUBSTRINGS" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "COUNT_SUBSTRING requires a substring")?;
            let needle = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after COUNT_SUBSTRING arguments",
            )?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    if needle.is_empty() {
                        return Err(ParseError("COUNT_SUBSTRING needle cannot be empty".into()));
                    }
                    ConstantValue::Integer(
                        i64::try_from(input.matches(&needle).count()).map_err(|_| {
                            ParseError("COUNT_SUBSTRING count exceeds Int64".into())
                        })?,
                    )
                }
                _ => ConstantValue::Null,
            }
        }
        "PATH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "path requires one argument")?;
            match input {
                Some(input) => {
                    let suffix = if let Some((_, after_scheme)) = input.split_once("://") {
                        let start = after_scheme
                            .find(['/', '?', '#'])
                            .unwrap_or(after_scheme.len());
                        &after_scheme[start..]
                    } else {
                        input.as_str()
                    };
                    let end = suffix.find(['?', '#']).unwrap_or(suffix.len());
                    let path = &suffix[..end];
                    ConstantValue::Text(if path.starts_with('/') { path } else { "" }.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "NULLIF" => {
            let left = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "NULLIF requires a second argument")?;
            let right = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after NULLIF arguments")?;
            match (left, right) {
                (Some(left), Some(right)) if left == right => ConstantValue::Null,
                (Some(left), _) => ConstantValue::Text(left),
                (None, _) => ConstantValue::Null,
            }
        }
        "POSITIONCASEINSENSITIVE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "positionCaseInsensitive requires a needle",
            )?;
            let needle = text_arg(parser, depth + 1)?;
            let start = if parser.take(Token::Comma) {
                integer_arg(parser, depth + 1)?
            } else {
                Some(1)
            };
            require(
                parser,
                Token::RParen,
                "expected ')' after positionCaseInsensitive",
            )?;
            match (input, needle, start) {
                (Some(input), Some(needle), Some(start)) => {
                    if start < 0 {
                        return Err(ParseError(
                            "positionCaseInsensitive start must be nonnegative".into(),
                        ));
                    }
                    let offset = usize::try_from(start.max(1) - 1).unwrap_or(usize::MAX);
                    let position = if offset > input.len() {
                        0
                    } else if needle.is_empty() {
                        offset + 1
                    } else {
                        input.as_bytes()[offset..]
                            .windows(needle.len())
                            .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
                            .map_or(0, |index| offset + index + 1)
                    };
                    ConstantValue::Integer(i64::try_from(position).map_err(|_| {
                        ParseError("positionCaseInsensitive result exceeds Int64".into())
                    })?)
                }
                _ => ConstantValue::Null,
            }
        }
        "STRPOS" | "POSITIONUTF8" => {
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
        "RTRIM" => {
            let input = text_arg(parser, depth + 1)?;
            let characters = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(parser, Token::RParen, "expected ')' after RTRIM arguments")?;
            match (input, characters) {
                (Some(input), None) => ConstantValue::Text(input.trim_end_matches(' ').to_owned()),
                (Some(input), Some(Some(characters))) => ConstantValue::Text(
                    input
                        .trim_end_matches(|ch| characters.contains(ch))
                        .to_owned(),
                ),
                _ => ConstantValue::Null,
            }
        }
        "LAST_INDEX_OF" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "LAST_INDEX_OF requires a substring")?;
            let needle = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after LAST_INDEX_OF arguments",
            )?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    let index = input
                        .rfind(needle.as_str())
                        .map(|byte_index| input[..byte_index].chars().count() + 1)
                        .unwrap_or(0);
                    ConstantValue::Integer(
                        i64::try_from(index)
                            .map_err(|_| ParseError("LAST_INDEX_OF exceeds Int64".into()))?,
                    )
                }
                _ => ConstantValue::Null,
            }
        }
        "COLLAPSE_WHITESPACE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "COLLAPSE_WHITESPACE requires one argument",
            )?;
            match input {
                Some(value) => {
                    let mut result = String::new();
                    let mut pending_space = false;
                    for character in value.chars() {
                        if character.is_whitespace() {
                            pending_space = !result.is_empty();
                        } else {
                            if pending_space {
                                result.push(' ');
                                pending_space = false;
                            }
                            result.push(character);
                        }
                        if result.len() > MAX_TEXT_BYTES {
                            return Err(ParseError(
                                "COLLAPSE_WHITESPACE result is too large".into(),
                            ));
                        }
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "BTRIM" | "TRIMBOTH" => {
            let input = text_arg(parser, depth + 1)?;
            let characters = if parser.take(Token::Comma) {
                Some(text_arg(parser, depth + 1)?)
            } else {
                None
            };
            require(parser, Token::RParen, "expected ')' after BTRIM arguments")?;
            match (input, characters) {
                (Some(input), None) => ConstantValue::Text(input.trim_matches(' ').to_owned()),
                (Some(input), Some(Some(characters))) => ConstantValue::Text(
                    input
                        .trim_matches(|character| characters.contains(character))
                        .to_owned(),
                ),
                _ => ConstantValue::Null,
            }
        }
        "TRANSLATE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "TRANSLATE requires source characters")?;
            let from = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "TRANSLATE requires replacement characters",
            )?;
            let to = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after TRANSLATE arguments",
            )?;
            match (input, from, to) {
                (Some(input), Some(from), Some(to)) => {
                    let mut replacements = std::collections::HashMap::new();
                    let mut to_chars = to.chars();
                    for ch in from.chars() {
                        let replacement = to_chars.next();
                        replacements.entry(ch).or_insert(replacement);
                    }
                    let mut result = String::with_capacity(input.len());
                    for ch in input.chars() {
                        if let Some(replacement) = replacements.get(&ch) {
                            if let Some(mapped) = replacement {
                                if result.len() + mapped.len_utf8() > MAX_TEXT_BYTES {
                                    return Err(ParseError("TRANSLATE result is too large".into()));
                                }
                                result.push(*mapped);
                            }
                        } else {
                            if result.len() + ch.len_utf8() > MAX_TEXT_BYTES {
                                return Err(ParseError("TRANSLATE result is too large".into()));
                            }
                            result.push(ch);
                        }
                    }
                    ConstantValue::Text(result)
                }
                _ => ConstantValue::Null,
            }
        }
        "PATHFULL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "pathFull requires one argument")?;
            match input {
                Some(input) => {
                    let suffix = if let Some((_, after_scheme)) = input.split_once("://") {
                        let start = after_scheme
                            .find(['/', '?', '#'])
                            .unwrap_or(after_scheme.len());
                        &after_scheme[start..]
                    } else {
                        input.as_str()
                    };
                    ConstantValue::Text(suffix.to_owned())
                }
                None => ConstantValue::Null,
            }
        }
        "REPLACE" | "REPLACEALL" => {
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
        "HASTOKEN" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "hasToken requires a token")?;
            let token = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "expected ')' after hasToken")?;
            match (input, token) {
                (Some(input), Some(token)) => {
                    if token.is_empty()
                        || !token
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    {
                        return Err(ParseError("hasToken requires a single ASCII token".into()));
                    }
                    let found = input
                        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                        .any(|word| word == token);
                    ConstantValue::Integer(i64::from(found))
                }
                _ => ConstantValue::Null,
            }
        }
        "CONTAINS" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "CONTAINS requires a substring")?;
            let needle = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after CONTAINS arguments",
            )?;
            match (input, needle) {
                (Some(input), Some(needle)) => {
                    ConstantValue::Integer(i64::from(input.contains(needle.as_str())))
                }
                _ => ConstantValue::Null,
            }
        }
        "STARTS_WITH" | "STARTSWITH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "STARTS_WITH requires a prefix")?;
            let prefix = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after STARTS_WITH arguments",
            )?;
            match (input, prefix) {
                (Some(input), Some(prefix)) => {
                    ConstantValue::Integer(i64::from(input.starts_with(prefix.as_str())))
                }
                _ => ConstantValue::Null,
            }
        }
        "REPLACE_FIRST" | "REPLACEONE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "REPLACE_FIRST requires a search string",
            )?;
            let needle = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "REPLACE_FIRST requires a replacement")?;
            let replacement = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after REPLACE_FIRST arguments",
            )?;
            match (input, needle, replacement) {
                (Some(input), Some(needle), Some(replacement)) => {
                    if needle.is_empty() || !input.contains(&needle) {
                        ConstantValue::Text(input)
                    } else {
                        let length = input.len() - needle.len();
                        if length.saturating_add(replacement.len()) > MAX_TEXT_BYTES {
                            return Err(ParseError("REPLACE_FIRST result is too large".into()));
                        }
                        ConstantValue::Text(input.replacen(&needle, &replacement, 1))
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "STRIP_PREFIX" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "STRIP_PREFIX requires a prefix")?;
            let prefix = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after STRIP_PREFIX arguments",
            )?;
            match (input, prefix) {
                (Some(input), Some(prefix)) => ConstantValue::Text(
                    input
                        .strip_prefix(prefix.as_str())
                        .unwrap_or(input.as_str())
                        .to_owned(),
                ),
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
        "REPLACE_LAST" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "REPLACE_LAST requires a search string",
            )?;
            let needle = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "REPLACE_LAST requires a replacement")?;
            let replacement = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after REPLACE_LAST arguments",
            )?;
            match (input, needle, replacement) {
                (Some(input), Some(needle), Some(replacement)) => {
                    if needle.is_empty() {
                        ConstantValue::Text(input)
                    } else if let Some(index) = input.rfind(&needle) {
                        let length = (input.len() - needle.len())
                            .checked_add(replacement.len())
                            .ok_or_else(|| ParseError("REPLACE_LAST result is too large".into()))?;
                        if length > MAX_TEXT_BYTES {
                            return Err(ParseError("REPLACE_LAST result is too large".into()));
                        }
                        let mut result = String::with_capacity(length);
                        result.push_str(&input[..index]);
                        result.push_str(&replacement);
                        result.push_str(&input[index + needle.len()..]);
                        ConstantValue::Text(result)
                    } else {
                        ConstantValue::Text(input)
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "HASTOKENCASEINSENSITIVE" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "hasTokenCaseInsensitive requires a token",
            )?;
            let token = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after hasTokenCaseInsensitive",
            )?;
            match (input, token) {
                (Some(input), Some(token)) => {
                    if token.is_empty()
                        || !token
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    {
                        return Err(ParseError(
                            "hasTokenCaseInsensitive requires a single ASCII token".into(),
                        ));
                    }
                    let found = input
                        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                        .any(|word| word.eq_ignore_ascii_case(&token));
                    ConstantValue::Integer(i64::from(found))
                }
                _ => ConstantValue::Null,
            }
        }
        "ENDS_WITH" | "ENDSWITH" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "ENDS_WITH requires a suffix")?;
            let suffix = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after ENDS_WITH arguments",
            )?;
            match (input, suffix) {
                (Some(input), Some(suffix)) => {
                    ConstantValue::Integer(i64::from(input.ends_with(suffix.as_str())))
                }
                _ => ConstantValue::Null,
            }
        }
        "URL_DECODE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "URL_DECODE requires one argument")?;
            match input {
                Some(input) => {
                    let source = input.as_bytes();
                    let mut result = Vec::with_capacity(source.len());
                    let mut index = 0;
                    while index < source.len() {
                        if source[index] == b'%' {
                            if index + 2 >= source.len() {
                                return Err(ParseError("URL_DECODE has incomplete escape".into()));
                            }
                            let high = char::from(source[index + 1]).to_digit(16);
                            let low = char::from(source[index + 2]).to_digit(16);
                            let byte = match (high, low) {
                                (Some(high), Some(low)) => (high * 16 + low) as u8,
                                _ => {
                                    return Err(ParseError("URL_DECODE has invalid escape".into()));
                                }
                            };
                            result.push(byte);
                            index += 3;
                        } else {
                            result.push(source[index]);
                            index += 1;
                        }
                    }
                    ConstantValue::Text(
                        String::from_utf8(result)
                            .map_err(|_| ParseError("URL_DECODE result is not UTF-8".into()))?,
                    )
                }
                None => ConstantValue::Null,
            }
        }
        "BASE64_ENCODE" | "BASE64ENCODE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "BASE64_ENCODE requires one argument")?;
            match input {
                Some(value) => {
                    let output_len = value
                        .len()
                        .checked_add(2)
                        .and_then(|length| length.checked_div(3))
                        .and_then(|groups| groups.checked_mul(4))
                        .ok_or_else(|| ParseError("BASE64_ENCODE result is too large".into()))?;
                    if output_len > MAX_TEXT_BYTES {
                        return Err(ParseError("BASE64_ENCODE result is too large".into()));
                    }
                    const ALPHABET: &[u8; 64] =
                        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
                    let mut result = String::with_capacity(output_len);
                    for chunk in value.as_bytes().chunks(3) {
                        let mut bits = u32::from(chunk[0]) << 16;
                        if chunk.len() > 1 {
                            bits |= u32::from(chunk[1]) << 8;
                        }
                        if chunk.len() > 2 {
                            bits |= u32::from(chunk[2]);
                        }
                        result.push(char::from(ALPHABET[((bits >> 18) & 63) as usize]));
                        result.push(char::from(ALPHABET[((bits >> 12) & 63) as usize]));
                        if chunk.len() > 1 {
                            result.push(char::from(ALPHABET[((bits >> 6) & 63) as usize]));
                        } else {
                            result.push('=');
                        }
                        if chunk.len() > 2 {
                            result.push(char::from(ALPHABET[(bits & 63) as usize]));
                        } else {
                            result.push('=');
                        }
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
            }
        }
        "STRIP_SUFFIX" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "STRIP_SUFFIX requires a suffix")?;
            let suffix = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after STRIP_SUFFIX arguments",
            )?;
            match (input, suffix) {
                (Some(input), Some(suffix)) => ConstantValue::Text(
                    input
                        .strip_suffix(suffix.as_str())
                        .unwrap_or(input.as_str())
                        .to_owned(),
                ),
                _ => ConstantValue::Null,
            }
        }
        "NETLOC" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "netloc requires one argument")?;
            match input {
                Some(input) => {
                    let authority = input.split_once("://").map_or("", |(_, rest)| {
                        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
                        &rest[..end]
                    });
                    ConstantValue::Text(authority.to_owned())
                }
                None => ConstantValue::Null,
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
        "SPACE" => {
            let count = integer_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "SPACE requires one argument")?;
            match count {
                Some(count) => {
                    let width = usize::try_from(count)
                        .map_err(|_| ParseError("SPACE count must be nonnegative".into()))?;
                    if width > MAX_TEXT_BYTES {
                        return Err(ParseError("SPACE result is too large".into()));
                    }
                    ConstantValue::Text(" ".repeat(width))
                }
                None => ConstantValue::Null,
            }
        }
        "TRY_TO_INT64" | "TOINT64ORNULL" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "TRY_TO_INT64 requires one argument")?;
            match input {
                Some(value) => value
                    .parse::<i64>()
                    .map_or(ConstantValue::Null, ConstantValue::Integer),
                None => ConstantValue::Null,
            }
        }
        "APPENDTRAILINGCHARIFABSENT" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "appendTrailingCharIfAbsent requires a character",
            )?;
            let ending = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after appendTrailingCharIfAbsent",
            )?;
            match (input, ending) {
                (Some(mut input), Some(ending)) => {
                    if ending.chars().count() != 1 {
                        return Err(ParseError(
                            "appendTrailingCharIfAbsent requires exactly one character".into(),
                        ));
                    }
                    if !input.is_empty() && !input.ends_with(&ending) {
                        if input.len().saturating_add(ending.len()) > MAX_TEXT_BYTES {
                            return Err(ParseError(
                                "appendTrailingCharIfAbsent result is too large".into(),
                            ));
                        }
                        input.push_str(&ending);
                    }
                    ConstantValue::Text(input)
                }
                _ => ConstantValue::Null,
            }
        }
        "RPAD" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "RPAD requires a width")?;
            let width = integer_arg(parser, depth + 1)?;
            let padding = if parser.take(Token::Comma) {
                text_arg(parser, depth + 1)?
            } else {
                Some(" ".to_owned())
            };
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
        "BASE64_DECODE" | "BASE64DECODE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "BASE64_DECODE requires one argument")?;
            match input {
                Some(value) => {
                    fn digit(value: u8) -> Option<u8> {
                        match value {
                            b'A'..=b'Z' => Some(value - b'A'),
                            b'a'..=b'z' => Some(value - b'a' + 26),
                            b'0'..=b'9' => Some(value - b'0' + 52),
                            b'+' => Some(62),
                            b'/' => Some(63),
                            _ => None,
                        }
                    }
                    if value.len() % 4 != 0 {
                        return Err(ParseError("BASE64_DECODE requires four-byte groups".into()));
                    }
                    let source = value.as_bytes();
                    let mut bytes = Vec::with_capacity(source.len() / 4 * 3);
                    for (index, group) in source.chunks_exact(4).enumerate() {
                        let last = index + 1 == source.len() / 4;
                        let a = digit(group[0])
                            .ok_or_else(|| ParseError("invalid BASE64_DECODE character".into()))?;
                        let b = digit(group[1])
                            .ok_or_else(|| ParseError("invalid BASE64_DECODE character".into()))?;
                        let (c, d, width) = if group[2] == b'=' {
                            if group[3] != b'=' || !last || b & 15 != 0 {
                                return Err(ParseError("invalid BASE64_DECODE padding".into()));
                            }
                            (0, 0, 1)
                        } else if group[3] == b'=' {
                            let c = digit(group[2]).ok_or_else(|| {
                                ParseError("invalid BASE64_DECODE character".into())
                            })?;
                            if !last || c & 3 != 0 {
                                return Err(ParseError("invalid BASE64_DECODE padding".into()));
                            }
                            (c, 0, 2)
                        } else {
                            let c = digit(group[2]).ok_or_else(|| {
                                ParseError("invalid BASE64_DECODE character".into())
                            })?;
                            let d = digit(group[3]).ok_or_else(|| {
                                ParseError("invalid BASE64_DECODE character".into())
                            })?;
                            (c, d, 3)
                        };
                        let bits = (u32::from(a) << 18)
                            | (u32::from(b) << 12)
                            | (u32::from(c) << 6)
                            | u32::from(d);
                        bytes.push((bits >> 16) as u8);
                        if width > 1 {
                            bytes.push((bits >> 8) as u8);
                        }
                        if width > 2 {
                            bytes.push(bits as u8);
                        }
                    }
                    ConstantValue::Text(
                        String::from_utf8(bytes)
                            .map_err(|_| ParseError("BASE64_DECODE result is not UTF-8".into()))?,
                    )
                }
                None => ConstantValue::Null,
            }
        }
        "EXTRACTURLPARAMETER" => {
            let input = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::Comma,
                "extractURLParameter requires a parameter name",
            )?;
            let name = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "expected ')' after extractURLParameter",
            )?;
            match (input, name) {
                (Some(input), Some(name)) => {
                    let before_fragment = input
                        .split_once('#')
                        .map_or(input.as_str(), |(head, _)| head);
                    let query = before_fragment
                        .split_once('?')
                        .map_or("", |(_, query)| query);
                    let value = query
                        .split('&')
                        .find_map(|part| {
                            if part == name {
                                Some("")
                            } else {
                                part.split_once('=')
                                    .and_then(|(key, value)| (key == name).then_some(value))
                            }
                        })
                        .unwrap_or("");
                    ConstantValue::Text(value.to_owned())
                }
                _ => ConstantValue::Null,
            }
        }
        "ISVALIDJSON" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "isValidJSON requires one argument")?;
            match input {
                Some(input) => {
                    ConstantValue::Integer(i64::from(crate::json::parse_json(&input).is_ok()))
                }
                None => ConstantValue::Null,
            }
        }
        "JSONHAS" | "JSONEXTRACTSTRING" | "JSONEXTRACTRAW" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::Comma, "JSON function requires a key")?;
            let path = text_arg(parser, depth + 1)?;
            require(
                parser,
                Token::RParen,
                "JSON function requires two arguments",
            )?;
            match (input, path) {
                (Some(input), Some(path)) => {
                    let json = crate::json::parse_json(&input)
                        .map_err(|error| ParseError(format!("invalid JSON: {error}")))?;
                    let found = json.at_path(&path);
                    match name.as_str() {
                        "JSONHAS" => ConstantValue::Integer(i64::from(found.is_some())),
                        "JSONEXTRACTSTRING" => ConstantValue::Text(
                            found
                                .and_then(|value| value.as_string())
                                .unwrap_or("")
                                .to_owned(),
                        ),
                        _ => ConstantValue::Text(
                            found.map(|value| value.raw()).unwrap_or("").to_owned(),
                        ),
                    }
                }
                _ => ConstantValue::Null,
            }
        }
        "JSON_QUOTE" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "JSON_QUOTE requires one argument")?;
            match input {
                Some(value) => {
                    let mut result = String::with_capacity(value.len().saturating_add(2));
                    result.push('"');
                    for character in value.chars() {
                        match character {
                            '"' => result.push_str("\\\""),
                            '\\' => result.push_str("\\\\"),
                            '\n' => result.push_str("\\n"),
                            '\r' => result.push_str("\\r"),
                            '\t' => result.push_str("\\t"),
                            '\u{0008}' => result.push_str("\\b"),
                            '\u{000C}' => result.push_str("\\f"),
                            character if character < '\u{0020}' => {
                                const HEX: &[u8; 16] = b"0123456789abcdef";
                                let byte = character as u8;
                                result.push_str("\\u00");
                                result.push(char::from(HEX[usize::from(byte >> 4)]));
                                result.push(char::from(HEX[usize::from(byte & 0x0f)]));
                            }
                            character => result.push(character),
                        }
                        if result.len() >= MAX_TEXT_BYTES {
                            return Err(ParseError("JSON_QUOTE result is too large".into()));
                        }
                    }
                    result.push('"');
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
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
        "LOWERUTF8" => {
            let input = text_arg(parser, depth + 1)?;
            require(parser, Token::RParen, "lowerUTF8 requires one argument")?;
            match input {
                Some(value) => {
                    let result = value.to_lowercase();
                    if result.len() > MAX_TEXT_BYTES {
                        return Err(ParseError("lowerUTF8 result is too large".into()));
                    }
                    ConstantValue::Text(result)
                }
                None => ConstantValue::Null,
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

#[cfg(test)]
mod clickhouse_readable_time_delta_tests {
    use crate::{parse_select, ConstantValue, Projection};
    #[test]
    fn formats_second_intervals_and_unit_limits() {
        for (sql, expected) in [
            ("SELECT formatReadableTimeDelta(0)", "0 seconds"),
            ("SELECT formatReadableTimeDelta(12345)", "3 hours, 25 minutes and 45 seconds"),
            ("SELECT formatReadableTimeDelta(100, 'minutes')", "1 minute and 40 seconds"),
            ("SELECT formatReadableTimeDelta(12345, 'minutes')", "205 minutes and 45 seconds"),
            ("SELECT formatReadableTimeDelta(-60)", "-1 minute"),
            ("SELECT formatReadableTimeDelta(3600, 'minutes', 'minutes')", "60 minutes"),
            ("SELECT formatReadableTimeDelta(5456456)", "2 months, 2 days, 3 hours, 40 minutes and 56 seconds"),
        ] {
            assert!(matches!(parse_select(sql).unwrap().projection, Projection::Constants(values)
                if values.len() == 1 && values[0].value == ConstantValue::Text(expected.into())), "{sql}");
        }
        assert!(parse_select("SELECT formatReadableTimeDelta(60, 'invalid')").is_err());
        assert!(parse_select("SELECT formatReadableTimeDelta(60, 'minutes', 'hours')").is_err());
    }
}
