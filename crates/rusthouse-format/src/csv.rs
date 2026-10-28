use std::io::{self, Write};

fn write_short_escaped_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    // Worst case: every byte is a doubled quote plus two delimiters.
    let mut buffer = [0u8; 130];
    let mut used = 1;
    buffer[0] = b'"';
    for &byte in field {
        buffer[used] = byte;
        used += 1;
        if byte == b'"' {
            buffer[used] = b'"';
            used += 1;
        }
    }
    buffer[used] = b'"';
    output.write_all(&buffer[..used + 1])
}

fn write_long_escaped_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    // Streaming a quote-heavy value one segment at a time can cause thousands
    // of tiny writes. Batch escaping on the stack without growing a String.
    let mut buffer = [0u8; 4096];
    let mut used = 1;
    buffer[0] = b'"';
    for &byte in field {
        if used + 2 > buffer.len() {
            output.write_all(&buffer[..used])?;
            used = 0;
        }
        buffer[used] = byte;
        used += 1;
        if byte == b'"' {
            buffer[used] = b'"';
            used += 1;
        }
    }
    if used == buffer.len() {
        output.write_all(&buffer)?;
        used = 0;
    }
    buffer[used] = b'"';
    output.write_all(&buffer[..used + 1])
}

/// Write one RFC 4180 quoted CSV field, doubling embedded quotes.
pub fn write_quoted_csv_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    if field.is_empty() {
        return output.write_all(b"\"\"");
    }
    // Short quote-free fields dominate tabular output. Emit both quotes and
    // the payload in a single write without a heap allocation.
    if field.len() <= 256 && !field.contains(&b'"') {
        let mut buffer = [0u8; 258];
        buffer[0] = b'"';
        buffer[1..field.len() + 1].copy_from_slice(field);
        buffer[field.len() + 1] = b'"';
        return output.write_all(&buffer[..field.len() + 2]);
    }
    if !field.contains(&b'"') {
        output.write_all(b"\"")?;
        output.write_all(field)?;
        return output.write_all(b"\"");
    }
    if field.len() <= 64 {
        write_short_escaped_field(output, field)
    } else {
        write_long_escaped_field(output, field)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_embedded_and_adjacent_quotes() {
        let mut output = Vec::new();
        write_quoted_csv_field(&mut output, b"a\"\"b").unwrap();
        assert_eq!(output, b"\"a\"\"\"\"b\"");
    }

    #[test]
    fn empty_field_requires_one_writer_call() {
        struct Calls(usize);
        impl Write for Calls {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0 += 1;
                assert_eq!(bytes, b"\"\"");
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut sink = Calls(0);
        write_quoted_csv_field(&mut sink, b"").unwrap();
        assert_eq!(sink.0, 1);
    }

    #[test]
    fn short_unescaped_field_is_emitted_with_one_write() {
        struct Calls {
            count: usize,
            data: Vec<u8>,
        }

        impl Write for Calls {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.count += 1;
                self.data.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut sink = Calls {
            count: 0,
            data: Vec::new(),
        };
        write_quoted_csv_field(&mut sink, b"hello").unwrap();
        assert_eq!(sink.count, 1);
        assert_eq!(sink.data, b"\"hello\"");
    }

    #[test]
    fn medium_plain_field_uses_one_write() {
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
        let plain = vec![b'x'; 200];
        let mut sink = Count {
            calls: 0,
            bytes: Vec::new(),
        };
        write_quoted_csv_field(&mut sink, &plain).unwrap();
        assert_eq!(sink.calls, 1);
        assert_eq!(sink.bytes.len(), 202);
        assert!(sink.bytes.starts_with(b"\"") && sink.bytes.ends_with(b"\""));
    }

    #[test]
    fn quotes_empty_and_plain_fields() {
        let mut output = Vec::new();
        write_quoted_csv_field(&mut output, b"").unwrap();
        write_quoted_csv_field(&mut output, b"plain").unwrap();
        assert_eq!(output, b"\"\"\"plain\"");
    }
    #[test]
    fn short_quote_heavy_fields_need_only_one_writer_call() {
        struct CountWrites {
            writes: usize,
            bytes: Vec<u8>,
        }
        impl Write for CountWrites {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut output = CountWrites {
            writes: 0,
            bytes: Vec::new(),
        };
        write_quoted_csv_field(&mut output, b"a\"\"b").unwrap();
        assert_eq!(output.writes, 1);
        assert_eq!(output.bytes, b"\"a\"\"\"\"b\"");
    }
}

#[cfg(test)]
mod quote_batching_tests {
    use super::*;
    struct Counted {
        writes: usize,
        bytes: Vec<u8>,
    }
    impl Write for Counted {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn quote_heavy_large_field_uses_bounded_writes() {
        let mut writer = Counted {
            writes: 0,
            bytes: Vec::new(),
        };
        let input = vec![b'"'; 10_000];
        write_quoted_csv_field(&mut writer, &input).unwrap();
        assert_eq!(writer.bytes.len(), 20_002);
        assert_eq!(writer.bytes.first(), Some(&b'"'));
        assert_eq!(writer.bytes.last(), Some(&b'"'));
        assert!(writer.bytes[1..20_001].iter().all(|&b| b == b'"'));
        assert!(writer.writes <= 6, "{} writes", writer.writes);
    }
    #[test]
    fn chunk_boundary_preserves_every_quote_and_binary_byte() {
        let input = [vec![b'a'; 4094], b"\"\\b\n".to_vec(), vec![b'"'; 5000]].concat();
        let mut output = Vec::new();
        write_quoted_csv_field(&mut output, &input).unwrap();
        let mut expected = vec![b'"'];
        for &byte in &input {
            expected.push(byte);
            if byte == b'"' {
                expected.push(b'"');
            }
        }
        expected.push(b'"');
        assert_eq!(output, expected);
    }
}
