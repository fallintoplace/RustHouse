use std::io::{self, Write};

/// Write one RFC 4180 quoted CSV field, doubling embedded quotes.
pub fn write_quoted_csv_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    if field.is_empty() {
        return output.write_all(b"\"\"");
    }
    // Short quote-free fields dominate tabular output. Emit both quotes and
    // the payload in a single write without a heap allocation.
    if field.len() <= 64 && !field.contains(&b'"') {
        let mut buffer = [0u8; 66];
        buffer[0] = b'"';
        buffer[1..field.len() + 1].copy_from_slice(field);
        buffer[field.len() + 1] = b'"';
        return output.write_all(&buffer[..field.len() + 2]);
    }
    output.write_all(b"\"")?;
    let mut segments = field.split(|&byte| byte == b'"');
    output.write_all(segments.next().unwrap_or_default())?;
    for segment in segments {
        output.write_all(b"\"\"")?;
        output.write_all(segment)?;
    }
    output.write_all(b"\"")
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
    fn quotes_empty_and_plain_fields() {
        let mut output = Vec::new();
        write_quoted_csv_field(&mut output, b"").unwrap();
        write_quoted_csv_field(&mut output, b"plain").unwrap();
        assert_eq!(output, b"\"\"\"plain\"");
    }
}
