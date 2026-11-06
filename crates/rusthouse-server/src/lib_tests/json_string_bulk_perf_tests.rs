use super::*;

struct CountingWriter {
    writes: usize,
    bytes: Vec<u8>,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn encodes_long_plain_utf8_string_with_three_writes() {
    let value = "café".repeat(4096);
    let mut output = CountingWriter {
        writes: 0,
        bytes: Vec::new(),
    };
    write_json_string(&mut output, &value).unwrap();
    assert_eq!(output.writes, 3);
    assert_eq!(output.bytes.len(), value.len() + 2);
    assert_eq!(&output.bytes[1..1 + value.len()], value.as_bytes());
}

#[test]
fn preserves_all_json_escape_variants() {
    let mut output = Vec::new();
    write_json_string(&mut output, "a\"b\\\n\r\t\u{08}\u{0c}\u{0001}é").unwrap();
    assert_eq!(output, "\"a\\\"b\\\\\\n\\r\\t\\b\\f\\u0001é\"".as_bytes());
}
