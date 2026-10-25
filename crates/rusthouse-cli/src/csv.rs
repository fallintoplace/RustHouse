use std::io::{self, Write};

const MAX_FIELD_BYTES: usize = 1024 * 1024;

/// Convert RustHouse's numeric tab-separated output into RFC 4180-style CSV.
pub struct CsvWriter<W: Write> {
    output: W,
    field: Vec<u8>,
}

impl<W: Write> CsvWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            field: Vec::new(),
        }
    }

    fn finish_field(&mut self, separator: &[u8]) -> io::Result<()> {
        let needs_quotes = self
            .field
            .iter()
            .any(|byte| matches!(byte, b',' | b'"' | b'\r' | b'\n'));
        if needs_quotes {
            self.output.write_all(b"\"")?;
        }
        for &byte in &self.field {
            if byte == b'"' {
                self.output.write_all(b"\"\"")?;
            } else {
                self.output.write_all(&[byte])?;
            }
        }
        if needs_quotes {
            self.output.write_all(b"\"")?;
        }
        self.field.clear();
        self.output.write_all(separator)
    }
}

impl<W: Write> Write for CsvWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for &byte in buf {
            match byte {
                b'\t' => self.finish_field(b",")?,
                b'\n' => self.finish_field(b"\r\n")?,
                _ => {
                    if self.field.len() >= MAX_FIELD_BYTES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "CSV field exceeds 1 MiB",
                        ));
                    }
                    self.field.push(byte);
                }
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_split_rows_and_escapes_fields() {
        let mut output = Vec::new();
        {
            let mut csv = CsvWriter::new(&mut output);
            csv.write_all(b"left\tright\n1\t").unwrap();
            csv.write_all(b"2\n\"hello, world\"\t4\n").unwrap();
            csv.flush().unwrap();
        }
        assert_eq!(
            output,
            b"left,right\r\n1,2\r\n\"\"\"hello, world\"\"\",4\r\n"
        );
    }

    #[test]
    fn rejects_oversized_fields() {
        let mut csv = CsvWriter::new(Vec::new());
        assert!(csv.write_all(&vec![b'a'; MAX_FIELD_BYTES + 1]).is_err());
    }

    #[test]
    fn flush_does_not_split_a_quoted_field() {
        let mut csv = CsvWriter::new(Vec::new());
        csv.write_all(b"a,").unwrap();
        csv.flush().unwrap();
        csv.write_all(b"bc\n").unwrap();
        assert_eq!(csv.output, b"\"a,bc\"\r\n");
    }
}
