use std::io::{self, Write};

/// Write one RFC 4180 quoted CSV field, doubling embedded quotes.
pub fn write_quoted_csv_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    output.write_all(b"\"")?;
    for (index, segment) in field.split(|&byte| byte == b'"').enumerate() {
        if index != 0 {
            output.write_all(b"\"\"")?;
        }
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
    fn quotes_empty_and_plain_fields() {
        let mut output = Vec::new();
        write_quoted_csv_field(&mut output, b"").unwrap();
        write_quoted_csv_field(&mut output, b"plain").unwrap();
        assert_eq!(output, b"\"\"\"plain\"");
    }
}
