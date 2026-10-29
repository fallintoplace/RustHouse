use crate::tsv_escape::decode_tsv_field;
use rusthouse_format::write_quoted_csv_field as write_quoted_field;
use std::borrow::Cow;
use std::io::{self, Write};

const MAX_FIELD_BYTES: usize = 1024 * 1024;

/// Convert RustHouse's numeric tab-separated output into RFC 4180-style CSV.
pub struct CsvWriter<W: Write> {
    output: W,
    field: Vec<u8>,
    pending_cr: bool,
    columns: Option<usize>,
    fields_in_row: usize,
}

impl<W: Write> CsvWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            field: Vec::new(),
            pending_cr: false,
            columns: None,
            fields_in_row: 0,
        }
    }

    fn push_field_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_FIELD_BYTES - self.field.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CSV field exceeds 1 MiB",
            ));
        }
        self.field.extend_from_slice(bytes);
        Ok(())
    }

    fn push_field_byte(&mut self, byte: u8) -> io::Result<()> {
        self.push_field_bytes(&[byte])
    }

    /// Returns true when an LF has completed a previous CRLF row boundary.
    fn consume_pending_cr(&mut self, byte: u8) -> io::Result<bool> {
        if !self.pending_cr {
            return Ok(false);
        }
        self.pending_cr = false;
        if byte == b'\n' {
            self.finish_row()?;
            return Ok(true);
        }
        self.push_field_byte(b'\r')?;
        Ok(false)
    }

    fn finish_row(&mut self) -> io::Result<()> {
        let width = self.fields_in_row + 1;
        if self.columns.is_some_and(|columns| columns != width) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CSV row does not match header column count",
            ));
        }
        self.finish_field(b"\r\n")?;
        self.columns = Some(width);
        self.fields_in_row = 0;
        Ok(())
    }

    fn finish_field(&mut self, separator: &[u8]) -> io::Result<()> {
        write_csv_field(&mut self.output, &self.field)?;
        self.field.clear();
        self.output.write_all(separator)
    }
}

fn requires_csv_quoting(value: &[u8]) -> bool {
    value == b"\\N"
        || value
            .iter()
            .any(|byte| matches!(byte, b',' | b'"' | b'\t' | b'\r' | b'\n'))
}

fn write_csv_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    if field == b"\\N" {
        return output.write_all(b"\\N");
    }
    let decoded: Cow<'_, [u8]> = if field.contains(&b'\\') {
        Cow::Owned(decode_tsv_field(field)?)
    } else {
        Cow::Borrowed(field)
    };
    let value = decoded.as_ref();
    if !requires_csv_quoting(value) {
        return output.write_all(value);
    }
    write_quoted_field(output, value)
}

impl<W: Write> Write for CsvWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut index = 0;
        while index < buf.len() {
            if self.consume_pending_cr(buf[index])? {
                index += 1;
                continue;
            }
            match buf[index] {
                b'\r' => {
                    self.pending_cr = true;
                    index += 1;
                }
                b'\t' => {
                    self.finish_field(b",")?;
                    self.fields_in_row += 1;
                    index += 1;
                }
                b'\n' => {
                    self.finish_row()?;
                    index += 1;
                }
                _ => {
                    let chunk = &buf[index..];
                    let count = chunk
                        .iter()
                        .position(|byte| matches!(byte, b'\r' | b'\t' | b'\n'))
                        .unwrap_or(chunk.len());
                    self.push_field_bytes(&chunk[..count])?;
                    index += count;
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
    fn doubles_adjacent_quotes_as_one_csv_field() {
        let mut output = Vec::new();
        write_quoted_field(&mut output, b"\"\"").unwrap();
        assert_eq!(output, vec![b'"'; 6]);
    }

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
    fn treats_crlf_as_one_newline_even_across_writes() {
        let mut csv = CsvWriter::new(Vec::new());
        csv.write_all(b"left\tright\r").unwrap();
        csv.flush().unwrap();
        csv.write_all(b"\n1\t2\r\n").unwrap();
        assert_eq!(csv.output, b"left,right\r\n1,2\r\n");
    }

    #[test]
    fn keeps_bare_carriage_returns_inside_quoted_fields() {
        let mut csv = CsvWriter::new(Vec::new());
        csv.write_all(b"ab\rc\tother\n").unwrap();
        assert_eq!(csv.output, b"\"ab\rc\",other\r\n");

        let mut csv = CsvWriter::new(Vec::new());
        csv.write_all(b"ab\r").unwrap();
        csv.write_all(b"\tother\n").unwrap();
        assert_eq!(csv.output, b"\"ab\r\",other\r\n");
    }

    #[test]
    fn rejects_csv_rows_with_missing_or_extra_fields() {
        let mut missing = CsvWriter::new(Vec::new());
        missing.write_all(b"id\tname\n1\n").unwrap_err();
        assert_eq!(missing.output, b"id,name\r\n");

        let mut extra = CsvWriter::new(Vec::new());
        extra.write_all(b"id\tname\n1\tAlice\twrong\n").unwrap_err();
        assert_eq!(extra.columns, Some(2));

        let mut split = CsvWriter::new(Vec::new());
        split.write_all(b"id\tname\r").unwrap();
        split.write_all(b"\n1\tAlice\r\n").unwrap();
        assert_eq!(split.output, b"id,name\r\n1,Alice\r\n");
    }

    #[test]
    fn copies_large_plain_fields_across_boundaries() {
        let mut csv = CsvWriter::new(Vec::new());
        let plain = vec![b'x'; 32 * 1024];
        csv.write_all(&plain).unwrap();
        csv.write_all(b"\tend\r").unwrap();
        csv.write_all(b"\n").unwrap();

        let mut expected = plain;
        expected.extend_from_slice(b",end\r\n");
        assert_eq!(csv.output, expected);
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

    #[test]
    fn writes_large_fields_in_bulk_instead_of_one_byte_at_a_time() {
        struct CountingWriter {
            calls: usize,
            bytes: Vec<u8>,
        }

        impl Write for CountingWriter {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(data);
                Ok(data.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut csv = CsvWriter::new(CountingWriter {
            calls: 0,
            bytes: Vec::new(),
        });
        let field = vec![b'a'; 16 * 1024];
        csv.write_all(&field).unwrap();
        csv.write_all(b"\n").unwrap();
        assert_eq!(csv.output.calls, 2);
        assert_eq!(&csv.output.bytes[..field.len()], field);
        assert_eq!(&csv.output.bytes[field.len()..], b"\r\n");
    }
}

#[cfg(test)]
mod decoded_csv_regressions {
    use super::*;

    #[test]
    fn restores_sql_escapes_and_distinguishes_null_from_literal_marker() {
        let mut csv = CsvWriter::new(Vec::new());
        csv.write_all(b"value\nline\\nnext\ncol\\tval\n\\N\n\\\\N\n")
            .unwrap();
        assert_eq!(
            csv.output,
            b"value\r\n\"line\nnext\"\r\n\"col\tval\"\r\n\\N\r\n\"\\N\"\r\n"
        );
    }

    #[test]
    fn rejects_unknown_escape_sequences() {
        let mut csv = CsvWriter::new(Vec::new());
        csv.write_all(b"value\n").unwrap();
        assert_eq!(
            csv.write_all(b"bad\\q\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[test]
    fn borrows_plain_fields_and_keeps_csv_quote_rules() {
        let mut output = Vec::new();
        write_csv_field(&mut output, "café".as_bytes()).unwrap();
        write_csv_field(&mut output, b",comma").unwrap();
        write_csv_field(&mut output, b"a\\tb").unwrap();
        assert_eq!(output, "café\",comma\"\"a\tb\"".as_bytes());
    }
}
