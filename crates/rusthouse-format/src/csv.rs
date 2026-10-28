use std::io::{self, Write};

/// Write one RFC 4180 quoted CSV field, doubling embedded quotes.
pub fn write_quoted_csv_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    if field.is_empty() {
        return output.write_all(b"\"\"");
    }
    if !field.contains(&b'"') {
        output.write_all(b"\"")?;
        output.write_all(field)?;
        return output.write_all(b"\"");
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
    fn quotes_empty_and_plain_fields() {
        let mut output = Vec::new();
        write_quoted_csv_field(&mut output, b"").unwrap();
        write_quoted_csv_field(&mut output, b"plain").unwrap();
        assert_eq!(output, b"\"\"\"plain\"");
    }
}
