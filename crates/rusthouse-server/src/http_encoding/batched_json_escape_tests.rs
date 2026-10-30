use super::*;

struct Count {
    calls: usize,
    data: Vec<u8>,
}
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        self.data.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn large_quote_heavy_json_uses_bounded_writes() {
    let mut writer = Count {
        calls: 0,
        data: Vec::new(),
    };
    write_json_string(&mut writer, &"\"".repeat(10_000)).unwrap();
    assert_eq!(writer.data.len(), 20_002);
    assert!(writer.data.starts_with(br#""\""#));
    assert!(writer.data.ends_with(br#"\"""#));
    assert!(writer.calls <= 6, "{} writes", writer.calls);
}
#[test]
fn escapes_controls_without_changing_unicode_or_utf8_bytes() {
    let value = "caf\u{e9}\n\t\r\\\"\u{0000}\u{001f}\u{0008}\u{000c}";
    let mut output = Vec::new();
    write_json_string(&mut output, value).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\"caf\u{e9}\\n\\t\\r\\\\\\\"\\u0000\\u001f\\b\\f\""
    );
}
