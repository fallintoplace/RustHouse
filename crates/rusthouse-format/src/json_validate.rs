//! Bounded, allocation-free syntax check for complete JSONL objects.
//! Validate before emitting row delimiters to avoid publishing invalid JSON.

use std::io;

const MAX_JSON_NESTING: usize = 256;

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid JSON object")
}

struct Parser<'a> {
    input: &'a [u8],
    offset: usize,
}

impl Parser<'_> {
    fn whitespace(&mut self) {
        while self
            .input
            .get(self.offset)
            .is_some_and(|byte| matches!(*byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.offset += 1;
        }
    }

    fn eat(&mut self, expected: u8) -> bool {
        if self.input.get(self.offset) == Some(&expected) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn string(&mut self) -> io::Result<()> {
        if !self.eat(b'"') {
            return Err(invalid());
        }
        loop {
            let byte = *self.input.get(self.offset).ok_or_else(invalid)?;
            self.offset += 1;
            match byte {
                b'"' => return Ok(()),
                b'\\' => {
                    let escape = *self.input.get(self.offset).ok_or_else(invalid)?;
                    self.offset += 1;
                    if escape == b'u' {
                        for _ in 0..4 {
                            let digit = *self.input.get(self.offset).ok_or_else(invalid)?;
                            if !digit.is_ascii_hexdigit() {
                                return Err(invalid());
                            }
                            self.offset += 1;
                        }
                    } else if !matches!(escape, b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') {
                        return Err(invalid());
                    }
                }
                0..=31 => return Err(invalid()),
                _ => {}
            }
        }
    }

    fn number(&mut self) -> io::Result<()> {
        self.eat(b'-');
        if !self.eat(b'0') {
            let first = *self.input.get(self.offset).ok_or_else(invalid)?;
            if !matches!(first, b'1'..=b'9') {
                return Err(invalid());
            }
            self.offset += 1;
            self.digits();
        }
        if self.eat(b'.') && self.digits() == 0 {
            return Err(invalid());
        }
        if self.eat(b'e') || self.eat(b'E') {
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if self.digits() == 0 {
                return Err(invalid());
            }
        }
        Ok(())
    }

    fn digits(&mut self) -> usize {
        let start = self.offset;
        while self
            .input
            .get(self.offset)
            .is_some_and(u8::is_ascii_digit)
        {
            self.offset += 1;
        }
        self.offset - start
    }

    fn keyword(&mut self, literal: &[u8]) -> io::Result<()> {
        if self.input.get(self.offset..self.offset.saturating_add(literal.len()))
            != Some(literal)
        {
            return Err(invalid());
        }
        self.offset += literal.len();
        Ok(())
    }

    fn object(&mut self, depth: usize) -> io::Result<()> {
        if depth > MAX_JSON_NESTING || !self.eat(b'{') {
            return Err(invalid());
        }
        self.whitespace();
        if self.eat(b'}') {
            return Ok(());
        }
        loop {
            self.string()?;
            self.whitespace();
            if !self.eat(b':') {
                return Err(invalid());
            }
            self.value(depth)?;
            self.whitespace();
            if self.eat(b'}') {
                return Ok(());
            }
            if !self.eat(b',') {
                return Err(invalid());
            }
            self.whitespace();
        }
    }

    fn array(&mut self, depth: usize) -> io::Result<()> {
        if depth > MAX_JSON_NESTING || !self.eat(b'[') {
            return Err(invalid());
        }
        self.whitespace();
        if self.eat(b']') {
            return Ok(());
        }
        loop {
            self.value(depth)?;
            self.whitespace();
            if self.eat(b']') {
                return Ok(());
            }
            if !self.eat(b',') {
                return Err(invalid());
            }
        }
    }

    fn value(&mut self, depth: usize) -> io::Result<()> {
        self.whitespace();
        match self.input.get(self.offset) {
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => self.string(),
            Some(b't') => self.keyword(b"true"),
            Some(b'f') => self.keyword(b"false"),
            Some(b'n') => self.keyword(b"null"),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(invalid()),
        }
    }
}

/// Accept exactly one complete UTF-8 JSON object, with bounded nesting.
pub fn validate_json_object(row: &[u8]) -> io::Result<()> {
    if !row.starts_with(b"{") || !row.ends_with(b"}") || std::str::from_utf8(row).is_err() {
        return Err(invalid());
    }
    let mut parser = Parser {
        input: row,
        offset: 0,
    };
    parser.object(1)?;
    if parser.offset != row.len() {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exact_json_objects() {
        for row in [
            br#"{}"#.as_slice(),
            br#"{"a":1,"b":[true,false,null,{"q":"\u00e9"}]}"#.as_slice(),
            r#"{"negative":-1.5E+10,"zero":0,"unicode":"é"}"#.as_bytes(),
            br#"{"escaped":"\\\/\"\b\f\n\r\t"}"#.as_slice(),
            br#"{"a": [1, 2, 3], "b": {"c": []}}"#.as_slice(),
        ] {
            validate_json_object(row).unwrap();
        }
    }

    #[test]
    fn rejects_incomplete_and_ambiguous_json_grammar() {
        for row in [
            br#"{"x":01}"#.as_slice(),
            br#"{"x":1.}"#.as_slice(),
            br#"{"x":1e+}"#.as_slice(),
            br#"{"x":NaN}"#.as_slice(),
            br#"{"x":true,}"#.as_slice(),
            br#"{"x":}"#.as_slice(),
            br#"{"x" 1}"#.as_slice(),
            br#"{"x":"\q"}"#.as_slice(),
            br#"{"x":"\u12G4"}"#.as_slice(),
            b"{\"x\":\"\n\"}".as_slice(),
            b"{\"x\":\"\xff\"}".as_slice(),
            br#"[1,2,3]"#.as_slice(),
            br#"{"x":1}{"y":2}"#.as_slice(),
            br#"{"x":[,]}"#.as_slice(),
        ] {
            assert_eq!(validate_json_object(row).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn rejects_excessive_json_nesting_before_stack_exhaustion() {
        let valid = format!("{{\"value\":{}0{}}}", "[".repeat(255), "]".repeat(255));
        validate_json_object(valid.as_bytes()).unwrap();
        let invalid = format!("{{\"value\":{}0{}}}", "[".repeat(256), "]".repeat(256));
        assert!(validate_json_object(invalid.as_bytes()).is_err());
    }
}
