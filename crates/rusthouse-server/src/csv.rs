//! Convert bounded SQL TSV results into RFC 4180-style CSV on the HTTP interface.
//! Decode TSV escapes before quoting values. NULL is represented as unquoted \\N.
use super::jsonl::decode_tsv_value;
use rusthouse_format::write_quoted_csv_field as write_quoted_field;
use std::borrow::Cow;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

pub(super) struct CsvWriter<W: Write> {
    output: W,
    line: Vec<u8>,
    encoded: Vec<u8>,
    columns: Option<usize>,
}

impl<W: Write> CsvWriter<W> {
    pub(super) fn new(output: W) -> Self {
        Self {
            output,
            line: Vec::new(),
            encoded: Vec::new(),
            columns: None,
        }
    }

    fn finish_row(&mut self) -> io::Result<()> {
        encode_complete_row(
            &mut self.output,
            &mut self.encoded,
            &mut self.columns,
            &self.line,
        )?;
        self.line.clear();
        Ok(())
    }
}

fn encode_complete_row<W: Write>(
    output: &mut W,
    encoded: &mut Vec<u8>,
    columns: &mut Option<usize>,
    line: &[u8],
) -> io::Result<()> {
    // Keep the encoded buffer: malformed CSV must never emit a partial row.
    encoded.clear();
    encoded.reserve(line.len().saturating_add(4));
    let mut field_count = 0usize;
    for field in line.split(|byte| *byte == b'\t') {
        if field_count != 0 {
            encoded.write_all(b",")?;
        }
        write_csv_field_from_tsv(encoded, field)?;
        field_count += 1;
    }
    if columns.is_some_and(|expected| expected != field_count) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "CSV row has a different column count",
        ));
    }
    encoded.write_all(b"\r\n")?;
    output.write_all(encoded)?;
    *columns = Some(field_count);
    Ok(())
}

fn write_csv_field_from_tsv<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    if field == b"\\N" {
        return output.write_all(b"\\N");
    }
    let value: Cow<'_, str> = if field.contains(&b'\\') {
        Cow::Owned(decode_tsv_value(field)?)
    } else {
        Cow::Borrowed(
            std::str::from_utf8(field)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 field"))?,
        )
    };
    let needs_quotes = value.as_ref() == "\\N"
        || value
            .bytes()
            .any(|byte| matches!(byte, b',' | b'"' | b'\t' | b'\n' | b'\r'));
    if !needs_quotes {
        return output.write_all(value.as_bytes());
    }
    write_quoted_field(output, value.as_bytes())
}

impl<W: Write> Write for CsvWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for chunk in buf.split_inclusive(|&byte| byte == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            let data = if complete {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            if data.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "CSV row exceeds 1 MiB",
                ));
            }
            if complete && self.line.is_empty() {
                encode_complete_row(
                    &mut self.output,
                    &mut self.encoded,
                    &mut self.columns,
                    data,
                )?;
                continue;
            }
            self.line.extend_from_slice(data);
            if complete {
                self.finish_row()?;
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
    fn contiguous_csv_rows_do_not_allocate_an_input_staging_buffer() {
        let mut writer = CsvWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        assert_eq!(writer.line.capacity(), 0);
        let value = "x".repeat(16 * 1024);
        writer.write_all(format!("{value}\n").as_bytes()).unwrap();
        assert_eq!(writer.line.capacity(), 0);
        assert!(writer.output.ends_with(format!("{value}\r\n").as_bytes()));
    }

    #[test]
    fn doubles_adjacent_quotes_in_quoted_csv_fields() {
        let mut output = Vec::new();
        write_quoted_field(&mut output, b"a\"\"b").unwrap();
        assert_eq!(output, b"\"a\"\"\"\"b\"");
    }

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
    fn quotes_decoded_tabs_as_one_csv_field() {
        let mut writer = CsvWriter::new(Vec::new());
        writer.write_all(b"value\nleft\\tright\n").unwrap();
        assert_eq!(writer.output, b"value\r\n\"left\tright\"\r\n");
    }

    #[test]
    fn writes_null_markers_and_plain_fields() {
        let mut output = Vec::new();
        write_csv_field_from_tsv(&mut output, b"\\N").unwrap();
        write_csv_field_from_tsv(&mut output, b"plain").unwrap();
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
        write_csv_field_from_tsv(&mut output, &field).unwrap();
        assert_eq!(output.calls, 1);
        assert_eq!(output.bytes, field);
    }

    #[test]
    fn invalid_late_field_does_not_write_partial_csv_row() {
        let mut writer = CsvWriter::new(Vec::new());
        writer.write_all(b"first\tsecond\n").unwrap();
        let good = writer.output.clone();
        assert_eq!(
            writer.write_all(b"ok\tbad\\q\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(writer.output, good);

        let mut writer = CsvWriter::new(Vec::new());
        assert!(writer.write_all(b"bad\\q\n").is_err());
        assert!(writer.output.is_empty());
        assert_eq!(writer.columns, None);
    }

    #[test]
    fn reuses_encoded_capacity_across_csv_rows() {
        let mut writer = CsvWriter::new(Vec::new());
        let row = vec![b'x'; 32 * 1024];
        writer.write_all(&row).unwrap();
        writer.write_all(b"\n").unwrap();
        let capacity = writer.encoded.capacity();
        writer.write_all(b"short\n").unwrap();
        assert_eq!(writer.encoded.capacity(), capacity);
        assert!(writer.output.ends_with(b"short\r\n"));
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
    #[test]
    fn streams_wide_rows_and_rejects_width_changes() {
        let header = std::iter::repeat_n("column", 256)
            .collect::<Vec<_>>()
            .join("\t");
        let row = std::iter::repeat_n("123", 256)
            .collect::<Vec<_>>()
            .join("\t");
        let mut writer = CsvWriter::new(Vec::new());
        writer
            .write_all(format!("{header}\n{row}\n").as_bytes())
            .unwrap();
        assert_eq!(
            writer.output.iter().filter(|&&byte| byte == b'\n').count(),
            2
        );
        assert_eq!(writer.columns, Some(256));
        assert!(writer.write_all(b"one\tcolumn\n").is_err());
    }

    #[test]
    fn csv_plain_utf8_fields_skip_escape_decoding() {
        let mut output = Vec::new();
        write_csv_field_from_tsv(&mut output, "café".as_bytes()).unwrap();
        assert_eq!(output, "café".as_bytes());
        output.clear();
        write_csv_field_from_tsv(&mut output, b"hello,world").unwrap();
        assert_eq!(output, b"\"hello,world\"");
        output.clear();
        assert!(write_csv_field_from_tsv(&mut output, b"\xff").is_err());
    }

    #[test]
    fn wide_plain_csv_rows_reuse_reserved_output_capacity() {
        let large = "x".repeat(4096);
        let mut writer = CsvWriter::new(Vec::new());
        writer
            .write_all(format!("value\n{large}\n").as_bytes())
            .unwrap();
        assert_eq!(writer.output, format!("value\r\n{large}\r\n").as_bytes());
        assert!(writer.encoded.capacity() >= large.len() + 4);
    }
}

#[cfg(test)]
mod single_pass_row_tests {
    use super::*;
    #[test]
    fn encodes_multi_column_rows_and_rejects_bad_width_without_leaking_output() {
        let mut csv = CsvWriter::new(Vec::new());
        csv.write_all(b"a\tb\n1\t2\n").unwrap();
        assert_eq!(csv.output, b"a,b\r\n1,2\r\n");
        assert_eq!(
            csv.write_all(b"only-one\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(csv.output, b"a,b\r\n1,2\r\n");
    }
}
