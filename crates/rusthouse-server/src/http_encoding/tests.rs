use super::*;

#[test]
fn short_plain_json_fields_use_one_write() {
    struct CountingWriter {
        calls: usize,
        bytes: Vec<u8>,
    }

    impl Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut writer = CountingWriter {
        calls: 0,
        bytes: Vec::new(),
    };
    write_json_string(&mut writer, "hello").unwrap();
    assert_eq!(writer.calls, 1);
    assert_eq!(writer.bytes, br#""hello""#);
}

#[test]
fn medium_json_string_uses_single_write() {
    struct Count {
        calls: usize,
        bytes: Vec<u8>,
    }
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let value = "é".repeat(100);
    let mut sink = Count {
        calls: 0,
        bytes: Vec::new(),
    };
    write_json_string(&mut sink, &value).unwrap();
    assert_eq!(sink.calls, 1);
    assert_eq!(sink.bytes, format!("\"{value}\"").as_bytes());
}

#[test]
fn parses_only_ascii_hex_digits() {
    assert_eq!(hex_digit(b'0'), Some(0));
    assert_eq!(hex_digit(b'9'), Some(9));
    assert_eq!(hex_digit(b'a'), Some(10));
    assert_eq!(hex_digit(b'F'), Some(15));
    assert_eq!(hex_digit(b'G'), None);
    assert_eq!(hex_digit(0xff), None);
    assert_eq!(decode_url("%4f%4A").unwrap(), "OJ");
}

#[test]
fn plus_only_queries_keep_utf8_without_percent_decoding() {
    assert_eq!(
        decode_url("SELECT+café+FROM+items").unwrap(),
        "SELECT café FROM items"
    );
    assert_eq!(decode_url("a+b%2Bc").unwrap(), "a b+c");
    assert!(decode_url(&"a+".repeat(MAX_QUERY_BYTES)).is_err());
}

#[test]
fn bounds_encoded_query_size_before_allocation() {
    let maximum = "%41".repeat(MAX_QUERY_BYTES);
    assert_eq!(decode_url(&maximum).unwrap().len(), MAX_QUERY_BYTES);
    assert!(decode_url(&format!("{maximum}%41")).is_err());
    assert!(decode_url(&("a".repeat(MAX_QUERY_BYTES * 4))).is_err());
}
