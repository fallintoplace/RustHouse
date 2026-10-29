//! Bounded, strict JSON parsing for constant SQL JSON scalar functions.
//! This is not the ClickHouse JSON column storage serialization.
const MAX_JSON_BYTES: usize = 1_048_576;
const MAX_JSON_DEPTH: usize = 32;

#[derive(Debug)]
pub(crate) struct JsonValue<'a> {
    raw: &'a str,
    kind: JsonKind<'a>,
}

#[derive(Debug)]
enum JsonKind<'a> {
    Primitive,
    Text(String),
    Array(Vec<JsonValue<'a>>),
    Object(Vec<(String, JsonValue<'a>)>),
}

impl<'a> JsonValue<'a> {
    pub(crate) fn raw(&self) -> &str {
        self.raw
    }

    pub(crate) fn as_string(&self) -> Option<&str> {
        match &self.kind {
            JsonKind::Text(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn at_path(&self, path: &str) -> Option<&JsonValue<'a>> {
        if path.is_empty() {
            return None;
        }
        let mut node = self;
        for segment in path.split('.') {
            node = match &node.kind {
                JsonKind::Object(fields) => fields
                    .iter()
                    .rev()
                    .find(|(name, _)| name == segment)
                    .map(|(_, value)| value)?,
                JsonKind::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(node)
    }
}

struct Parser<'a> {
    text: &'a str,
    offset: usize,
}

impl<'a> Parser<'a> {
    fn bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }

    fn next(&self) -> Option<u8> {
        self.bytes().get(self.offset).copied()
    }

    fn skip_ws(&mut self) {
        while self.next().is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n')) {
            self.offset += 1;
        }
    }

    fn consume_expected_byte_if_present(&mut self, expected: u8) -> bool {
        if self.next() == Some(expected) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), &'static str> {
        self.consume_expected_byte_if_present(expected)
            .then_some(())
            .ok_or("unexpected JSON character")
    }

    fn unicode_hex(&mut self) -> Result<u16, &'static str> {
        let mut value = 0u16;
        for _ in 0..4 {
            let byte = self.next().ok_or("incomplete Unicode escape")?;
            let digit = match byte {
                b'0'..=b'9' => u16::from(byte - b'0'),
                b'a'..=b'f' => u16::from(byte - b'a' + 10),
                b'A'..=b'F' => u16::from(byte - b'A' + 10),
                _ => return Err("invalid Unicode escape"),
            };
            value = value * 16 + digit;
            self.offset += 1;
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String, &'static str> {
        self.expect(b'"')?;
        // Plain JSON keys and values dominate. Find the closing quote in one
        // slice scan and allocate only the final string, without byte-by-byte
        // parser dispatch on every ordinary UTF-8 code unit.
        if let Some(end) = self.bytes()[self.offset..]
            .iter().position(|&byte| byte == b'"' || byte == b'\\' || byte < 0x20)
        {
            if self.bytes()[self.offset + end] == b'"' {
                let begin = self.offset;
                self.offset += end + 1;
                return Ok(self.text[begin..begin + end].to_owned());
            }
        }
        let mut output = String::new();
        let mut start = self.offset;
        loop {
            let byte = self.next().ok_or("unterminated JSON string")?;
            if byte == b'"' {
                output.push_str(&self.text[start..self.offset]);
                self.offset += 1;
                return Ok(output);
            }
            if byte < 0x20 {
                return Err("unescaped control character in JSON string");
            }
            if byte != b'\\' {
                self.offset += 1;
                continue;
            }
            output.push_str(&self.text[start..self.offset]);
            self.offset += 1;
            let escaped = self.next().ok_or("incomplete JSON escape")?;
            self.offset += 1;
            match escaped {
                b'"' => output.push('"'),
                b'\\' => output.push('\\'),
                b'/' => output.push('/'),
                b'b' => output.push('\u{0008}'),
                b'f' => output.push('\u{000C}'),
                b'n' => output.push('\n'),
                b'r' => output.push('\r'),
                b't' => output.push('\t'),
                b'u' => {
                    let first = self.unicode_hex()?;
                    let codepoint = if (0xD800..=0xDBFF).contains(&first) {
                        self.expect(b'\\')?;
                        self.expect(b'u')?;
                        let low = self.unicode_hex()?;
                        if !(0xDC00..=0xDFFF).contains(&low) {
                            return Err("invalid Unicode surrogate pair");
                        }
                        0x10000 + (u32::from(first - 0xD800) << 10) + u32::from(low - 0xDC00)
                    } else {
                        if (0xDC00..=0xDFFF).contains(&first) {
                            return Err("isolated low Unicode surrogate");
                        }
                        u32::from(first)
                    };
                    output.push(char::from_u32(codepoint).ok_or("invalid Unicode scalar")?);
                }
                _ => return Err("invalid JSON string escape"),
            }
            start = self.offset;
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.offset;
        while self.next().is_some_and(|byte| byte.is_ascii_digit()) {
            self.offset += 1;
        }
        self.offset - start
    }

    fn number(&mut self) -> Result<(), &'static str> {
        self.consume_expected_byte_if_present(b'-');
        match self.next() {
            Some(b'0') => self.offset += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err("invalid JSON number"),
        }
        if self.consume_expected_byte_if_present(b'.') && self.digits() == 0 {
            return Err("missing JSON fractional digits");
        }
        if self.consume_expected_byte_if_present(b'e')
            || self.consume_expected_byte_if_present(b'E')
        {
            if !self.consume_expected_byte_if_present(b'+') {
                self.consume_expected_byte_if_present(b'-');
            }
            if self.digits() == 0 {
                return Err("missing JSON exponent digits");
            }
        }
        Ok(())
    }

    fn value(&mut self, depth: usize) -> Result<JsonValue<'a>, &'static str> {
        if depth > MAX_JSON_DEPTH {
            return Err("JSON nesting is too deep");
        }
        self.skip_ws();
        let begin = self.offset;
        let kind = match self.next() {
            Some(b'"') => JsonKind::Text(self.string()?),
            Some(b'{') => {
                self.offset += 1;
                self.skip_ws();
                let mut fields = Vec::new();
                if !self.consume_expected_byte_if_present(b'}') {
                    loop {
                        let key = self.string()?;
                        self.skip_ws();
                        self.expect(b':')?;
                        fields.push((key, self.value(depth + 1)?));
                        self.skip_ws();
                        if self.consume_expected_byte_if_present(b'}') {
                            break;
                        }
                        self.expect(b',')?;
                        self.skip_ws();
                    }
                }
                JsonKind::Object(fields)
            }
            Some(b'[') => {
                self.offset += 1;
                self.skip_ws();
                let mut items = Vec::new();
                if !self.consume_expected_byte_if_present(b']') {
                    loop {
                        items.push(self.value(depth + 1)?);
                        self.skip_ws();
                        if self.consume_expected_byte_if_present(b']') {
                            break;
                        }
                        self.expect(b',')?;
                    }
                }
                JsonKind::Array(items)
            }
            Some(b't' | b'f' | b'n') => {
                let rest = &self.bytes()[self.offset..];
                let literal = if rest.starts_with(b"true") {
                    b"true".as_slice()
                } else if rest.starts_with(b"false") {
                    b"false".as_slice()
                } else if rest.starts_with(b"null") {
                    b"null".as_slice()
                } else {
                    return Err("invalid JSON literal");
                };
                self.offset += literal.len();
                JsonKind::Primitive
            }
            Some(b'-' | b'0'..=b'9') => {
                self.number()?;
                JsonKind::Primitive
            }
            _ => return Err("expected JSON value"),
        };
        Ok(JsonValue {
            raw: &self.text[begin..self.offset],
            kind,
        })
    }
}

pub(crate) fn parse_json(text: &str) -> Result<JsonValue<'_>, &'static str> {
    if text.len() > MAX_JSON_BYTES {
        return Err("JSON exceeds 1 MiB");
    }
    let mut parser = Parser { text, offset: 0 };
    let value = parser.value(0)?;
    parser.skip_ws();
    if parser.offset != text.len() {
        return Err("extra bytes after JSON value");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::parse_json;

    #[test]
    fn fast_plain_json_strings_preserve_non_ascii_and_escaped_fallback() {
        let parsed = parse_json(r#"{"café":"zażółć","escaped":"a\nb"}"#).unwrap();
        assert_eq!(parsed.at_path("café").and_then(|value| value.as_string()), Some("zażółć"));
        assert_eq!(parsed.at_path("escaped").and_then(|value| value.as_string()), Some("a\nb"));
        assert!(parse_json("{\"bad\":\"unterminated").is_err());
    }

    #[test]
    fn parses_nested_objects_arrays_and_escapes() {
        let json = r#"{"items":[{"name":"rocket \uD83D\uDE80"},null],"n":-1.2e+3}"#;
        let value = parse_json(json).unwrap();
        assert_eq!(
            value.at_path("items.0.name").unwrap().as_string(),
            Some("rocket 🚀")
        );
        assert_eq!(value.at_path("n").unwrap().raw(), "-1.2e+3");
        assert!(value.at_path("items.1.name").is_none());
    }

    #[test]
    fn rejects_invalid_json_numbers_strings_and_structure() {
        for input in [
            "",
            "{",
            "true false",
            "01",
            "+1",
            "1.",
            "1e",
            "--1",
            r#"{"bad":}"#,
            r#"{"a":1,}"#,
            "[1,]",
            r#""bad\q""#,
            r#""\uD800""#,
            r#""\uDC00""#,
            r#"{"a":1} trailing"#,
            "{'single':1}",
            "NaN",
        ] {
            assert!(parse_json(input).is_err(), "{input}");
        }
        assert!(parse_json(&"[".repeat(40)).is_err());
    }

    #[test]
    fn only_json_whitespace_is_accepted_between_tokens() {
        assert!(parse_json(" \t\r\n true \n").is_ok());
        for input in ["\u{000b}true", "true\u{000c}", "[1,\u{001f}2]"] {
            assert!(parse_json(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn respects_text_cap_and_last_duplicate_key() {
        assert!(parse_json(&" ".repeat(1_048_577)).is_err());
        let value = parse_json(r#"{"x":1,"x":2}"#).unwrap();
        assert_eq!(value.at_path("x").unwrap().raw(), "2");
    }
}
