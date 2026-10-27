//! Convert bounded TSV query results into newline-delimited JSON objects.
//! Numeric values remain strings to preserve Int128 and UInt128 precision.
use super::write_json_string;
use rusthouse_format::decode_tsv_field;
use std::borrow::Cow;
use std::collections::HashSet;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(super) fn decode_tsv_value(value: &[u8]) -> io::Result<String> {
    String::from_utf8(decode_tsv_field(value)?)
        .map_err(|_| invalid("JSONL field is not UTF-8"))
}

fn parse_headers(line: &[u8]) -> io::Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut headers = Vec::new();
    for header in line.split(|byte| *byte == b'\t') {
        let name = std::str::from_utf8(header).map_err(|_| invalid("JSONL header is not UTF-8"))?;
        if name.is_empty() {
            return Err(invalid("JSONL header name must not be empty"));
        }
        if headers.len() >= 64 {
            return Err(invalid("JSONL supports at most 64 columns"));
        }
        if !seen.insert(name) {
            return Err(invalid("JSONL needs unique projected column names"));
        }
        headers.push(name.to_owned());
    }
    Ok(headers)
}

fn decoded_row_field(field: &[u8]) -> io::Result<Cow<'_, str>> {
    if field.contains(&b'\\') {
        decode_tsv_value(field).map(Cow::Owned)
    } else {
        std::str::from_utf8(field)
            .map(Cow::Borrowed)
            .map_err(|_| invalid("JSONL field is not UTF-8"))
    }
}

fn write_json_row<W: Write>(output: &mut W, headers: &[String], line: &[u8]) -> io::Result<()> {
    if 1 + line.iter().filter(|byte| **byte == b'\t').count() != headers.len() {
        return Err(invalid("JSONL row does not match header column count"));
    }
    let mut row = Vec::with_capacity(line.len() + headers.len() * 8 + 3);
    row.write_all(b"{")?;
    for (index, (header, field)) in headers
        .iter()
        .zip(line.split(|byte| *byte == b'\t'))
        .enumerate()
    {
        if index != 0 {
            row.write_all(b",")?;
        }
        write_json_string(&mut row, header)?;
        row.write_all(b":")?;
        if field == b"\\N" {
            row.write_all(b"null")?;
        } else {
            let text = decoded_row_field(field)?;
            write_json_string(&mut row, &text)?;
        }
    }
    row.write_all(b"}\n")?;
    output.write_all(&row)
}

pub struct JsonlWriter<W: Write> {
    output: W,
    headers: Option<Vec<String>>,
    line: Vec<u8>,
}

impl<W: Write> JsonlWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            headers: None,
            line: Vec::new(),
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        if let Some(headers) = &self.headers {
            let result = write_json_row(&mut self.output, headers, &self.line);
            self.line.clear();
            return result;
        }
        let headers = parse_headers(&self.line);
        self.line.clear();
        self.headers = Some(headers?);
        Ok(())
    }
}

impl<W: Write> Write for JsonlWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for chunk in bytes.split_inclusive(|&byte| byte == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            let fragment = if complete {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            if fragment.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
                return Err(invalid("JSONL row exceeds 1 MiB"));
            }
            self.line.extend_from_slice(fragment);
            if complete {
                self.finish_line()?;
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_additional_tsv_control_escapes() {
        assert_eq!(
            decode_tsv_value(br"\b\f\v").unwrap().as_bytes(),
            b"\x08\x0c\x0b"
        );
    }

    #[test]
    fn borrows_plain_fields_and_decodes_only_escaped_fields() {
        let plain = decoded_row_field("café".as_bytes()).unwrap();
        assert!(matches!(plain, Cow::Borrowed("café")));
        let escaped = decoded_row_field(br"caf\xC3\xA9").unwrap();
        assert!(matches!(escaped, Cow::Owned(ref value) if value == "café"));
        assert!(decoded_row_field(b"\\q").is_err());
    }

    #[test]
    fn rejects_empty_headers_and_more_than_64_columns() {
        assert!(parse_headers(b"\tname").is_err());
        let names = (0..65)
            .map(|index| format!("c{index}"))
            .collect::<Vec<_>>()
            .join("\t");
        assert_eq!(
            parse_headers(names.as_bytes()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn batches_multiple_rows_and_fragmented_tail() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"col\none\ntw").unwrap();
        writer.write_all(b"o\n").unwrap();
        assert_eq!(writer.output, b"{\"col\":\"one\"}\n{\"col\":\"two\"}\n");
    }

    #[test]
    fn accepts_exact_max_length_and_rejects_excess_row() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"col\n").unwrap();
        let oversized = vec![b'x'; MAX_LINE_BYTES + 1];
        assert_eq!(
            writer.write_all(&oversized).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn jsonl_phases_validate_headers_before_writing_rows() {
        assert_eq!(
            parse_headers(b"left\tright").unwrap(),
            vec!["left".to_owned(), "right".to_owned()]
        );
        assert_eq!(
            parse_headers(b"left\tleft").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut output = Vec::new();
        write_json_row(&mut output, &["key".to_owned()], b"hello\\nworld").unwrap();
        assert_eq!(output, b"{\"key\":\"hello\\nworld\"}\n");
    }

    #[test]
    fn decodes_sql_tsv_escaping_and_preserves_large_numbers() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"label\tamount\tmissing\n").unwrap();
        writer
            .write_all(b"hello\\tworld\t340282366920938463463374607431768211455\t\\N\n")
            .unwrap();
        writer.write_all(b"back\\\\slash\tNULL\t\\N\n").unwrap();
        let output = String::from_utf8(writer.output).unwrap();
        assert_eq!(
            output,
            "{\"label\":\"hello\\tworld\",\"amount\":\"340282366920938463463374607431768211455\",\"missing\":null}\n{\"label\":\"back\\\\slash\",\"amount\":\"NULL\",\"missing\":null}\n"
        );
    }

    #[test]
    fn rejects_duplicate_columns_malformed_rows_and_invalid_escapes() {
        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(b"a\ta\n").is_err());

        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"a\tb\n").unwrap();
        assert!(writer.write_all(b"1\n").is_err());

        for invalid in [&b"bad\\q"[..], &b"bad\\xG0"[..], &b"bad\\x"[..]] {
            assert!(decode_tsv_value(invalid).is_err());
        }
        assert_eq!(
            decode_tsv_value(br"\xFF").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            decode_tsv_value(b"quoted\\x22word").unwrap(),
            "quoted\"word"
        );
    }

    #[test]
    fn invalid_late_field_does_not_emit_partial_json() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"left\tright\n1\t2\n").unwrap();
        let good = writer.output.clone();
        assert_eq!(
            writer.write_all(b"ok\tbad\\q\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(writer.output, good);
    }

    #[test]
    fn failed_header_keeps_writer_ready_for_new_header() {
        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(b"key\tkey\n").is_err());
        writer.write_all(b"key\nvalue\n").unwrap();
        assert_eq!(writer.output, b"{\"key\":\"value\"}\n");
    }

    #[test]
    fn handles_fragmented_writes_and_bounded_lines() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"key").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"\na").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"bc\n").unwrap();
        assert_eq!(
            String::from_utf8(writer.output).unwrap(),
            "{\"key\":\"abc\"}\n"
        );

        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(&vec![b'x'; MAX_LINE_BYTES + 1]).is_err());
    }
    #[test]
    fn plain_and_escaped_json_fields_produce_identical_values() {
        let mut output = Vec::new();
        write_json_row(&mut output, &["value".to_owned()], "café".as_bytes()).unwrap();
        write_json_row(&mut output, &["value".to_owned()], b"a\\tb").unwrap();
        assert_eq!(
            output,
            "{\"value\":\"café\"}\n{\"value\":\"a\\tb\"}\n".as_bytes()
        );
        let mut invalid = Vec::new();
        assert!(write_json_row(&mut invalid, &["value".to_owned()], &[0xff]).is_err());
    }
}
