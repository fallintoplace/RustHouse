//! Convert bounded SQL TSV results into RFC 4180-style CSV on the HTTP interface.
//! Decode TSV escapes before quoting values. NULL is represented as unquoted \\N.
use super::jsonl::decode_tsv_value;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

pub(super) struct CsvWriter<W: Write> {
    output: W,
    line: Vec<u8>,
    columns: Option<usize>,
}

impl<W: Write> CsvWriter<W> {
    pub(super) fn new(output: W) -> Self {
        Self {
            output,
            line: Vec::new(),
            columns: None,
        }
    }

    fn finish_row(&mut self) -> io::Result<()> {
        let fields: Vec<&[u8]> = self.line.split(|byte| *byte == b'\t').collect();
        if let Some(columns) = self.columns {
            if columns != fields.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "CSV row has a different column count",
                ));
            }
        } else {
            self.columns = Some(fields.len());
        }
        for (index, field) in fields.into_iter().enumerate() {
            if index != 0 {
                self.output.write_all(b",")?;
            }
            write_csv_field(&mut self.output, field)?;
        }
        self.output.write_all(b"\r\n")?;
        self.line.clear();
        Ok(())
    }
}

fn write_csv_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    if field == b"\\N" {
        return output.write_all(b"\\N");
    }
    let value = decode_tsv_value(field)?;
    let needs_quotes = value == "\\N"
        || value
            .bytes()
            .any(|byte| matches!(byte, b',' | b'"' | b'\n' | b'\r'));
    if !needs_quotes {
        return output.write_all(value.as_bytes());
    }
    output.write_all(b"\"")?;
    let mut segments = value.as_bytes().split(|&byte| byte == b'"');
    if let Some(first) = segments.next() {
        output.write_all(first)?;
    }
    for segment in segments {
        output.write_all(b"\"\"")?;
        output.write_all(segment)?;
    }
    output.write_all(b"\"")
}

impl<W: Write> Write for CsvWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for &byte in buf {
            if byte == b'\n' {
                self.finish_row()?;
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "CSV row exceeds 1 MiB",
                    ));
                }
                self.line.push(byte);
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
    fn decodes_escaped_rows_and_quotes_csv_special_characters() {
        let mut output = Vec::new();
        {
            let mut csv = CsvWriter::new(&mut output);
            csv.write_all(b"name\tmessage\tmissing\n").unwrap();
            csv.write_all(b"service\tcomma,quote\"\n").unwrap_err(); // row width mismatch
        }
        assert_eq!(output, b"name,message,missing\r\n");

        let mut csv = CsvWriter::new(Vec::new());
        csv.write_all(b"message\tnull\n").unwrap();
        csv.write_all(b"hello\\tworld, \"yes\"\t\\N\n").unwrap();
        assert_eq!(
            csv.output,
            b"message,null\r\n\"hello\tworld, \"\"yes\"\"\",\\N\r\n"
        );
    }

    #[test]
    fn handles_fragmented_tsv_input_and_rejects_bad_escapes() {
        let mut csv = CsvWriter::new(Vec::new());
        csv.write_all(b"a\tb\nleft\\").unwrap();
        csv.write_all(b"nright\t42\n").unwrap();
        assert_eq!(csv.output, b"a,b\r\n\"left\nright\",42\r\n");

        let mut csv = CsvWriter::new(Vec::new());
        assert!(csv.write_all(b"column\ninvalid\\q\n").is_err());
    }

    #[test]
    fn writes_null_markers_and_plain_fields() {
        let mut output = Vec::new();
        write_csv_field(&mut output, b"\\N").unwrap();
        write_csv_field(&mut output, b"plain").unwrap();
        assert_eq!(output, b"\\Nplain");
    }

    #[test]
    fn writes_large_plain_fields_in_one_bulk_call() {
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
        let mut output = CountingWriter {
            calls: 0,
            bytes: Vec::new(),
        };
        let field = vec![b'x'; 32 * 1024];
        write_csv_field(&mut output, &field).unwrap();
        assert_eq!(output.calls, 1);
        assert_eq!(output.bytes, field);
    }

    #[test]
    fn caps_incomplete_rows_to_one_mib() {
        let mut csv = CsvWriter::new(Vec::new());
        assert!(csv.write_all(&vec![b'x'; MAX_LINE_BYTES + 1]).is_err());
    }
}

#[cfg(test)]
mod literal_null_marker_regressions {
    use super::*;

    #[test]
    fn distinguishes_literal_backslash_n_from_null_in_csv() {
        let mut csv = CsvWriter::new(Vec::new());
        csv.write_all(b"value\n\\N\n\\\\N\n").unwrap();
        assert_eq!(csv.output, b"value\r\n\\N\r\n\"\\N\"\r\n");
    }
}
