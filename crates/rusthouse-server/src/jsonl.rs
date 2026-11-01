//! Convert bounded TSV query results into newline-delimited JSON objects.
//! Numeric values remain strings to preserve Int128 and UInt128 precision.
use super::write_json_string;
use rusthouse_format::decode_tsv_field_into;
#[cfg(test)]
use rusthouse_format::decode_tsv_field;
#[cfg(test)]
use std::borrow::Cow;
use std::collections::HashSet;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_JSONL_COLUMNS: usize = 64;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
pub(super) fn decode_tsv_value(value: &[u8]) -> io::Result<String> {
    String::from_utf8(decode_tsv_field(value)?).map_err(|_| invalid("JSONL field is not UTF-8"))
}

fn parse_headers(line: &[u8]) -> io::Result<Vec<String>> {
    let mut headers = Vec::new();
    // Most SELECTs project no more than a handful of columns. Borrow
    // headers from the input line instead of allocating a hash table.
    let mut small_names = [""; 8];
    let mut seen: Option<HashSet<&str>> = None;
    for header in line.split(|byte| *byte == b'\t') {
        let name = std::str::from_utf8(header).map_err(|_| invalid("JSONL header is not UTF-8"))?;
        if name.is_empty() {
            return Err(invalid("JSONL header name must not be empty"));
        }
        let index = headers.len();
        if index >= MAX_JSONL_COLUMNS {
            return Err(invalid("JSONL supports at most 64 columns"));
        }
        let duplicate = if index < small_names.len() {
            if small_names[..index].contains(&name) {
                true
            } else {
                small_names[index] = name;
                false
            }
        } else {
            // Seed the large-schema set with the first eight borrowed names.
            // This catches duplicates across the stack/heap threshold.
            let seen = seen.get_or_insert_with(|| small_names.iter().copied().collect());
            !seen.insert(name)
        };
        if duplicate {
            return Err(invalid("JSONL needs unique projected column names"));
        }
        headers.push(name.to_owned());
    }
    Ok(headers)
}

#[cfg(test)]
fn decoded_row_field(field: &[u8]) -> io::Result<Cow<'_, str>> {
    if field.contains(&b'\\') {
        decode_tsv_value(field).map(Cow::Owned)
    } else {
        std::str::from_utf8(field)
            .map(Cow::Borrowed)
            .map_err(|_| invalid("JSONL field is not UTF-8"))
    }
}

fn row_width_error() -> io::Error {
    invalid("JSONL row does not match header column count")
}

fn write_json_row_into<W: Write>(
    output: &mut W,
    headers: &[Vec<u8>],
    line: &[u8],
    row: &mut Vec<u8>,
    decoded: &mut Vec<u8>,
) -> io::Result<()> {
    let mut fields = line.split(|byte| *byte == b'\t');
    row.clear();
    row.reserve(line.len() + headers.len() * 8 + 3);
    row.push(b'{');
    for (index, header) in headers.iter().enumerate() {
        let field = fields.next().ok_or_else(row_width_error)?;
        if index != 0 {
            row.push(b',');
        }
        row.extend_from_slice(header);
        row.push(b':');
        if field == b"\\N" {
            row.write_all(b"null")?;
        } else {
            // Decode escaped TSV bytes into one reusable field buffer.
            // Plain UTF-8 fields stay borrowed from the input line.
            let text = if field.contains(&b'\\') {
                decode_tsv_field_into(field, decoded)?;
                std::str::from_utf8(decoded)
                    .map_err(|_| invalid("JSONL field is not UTF-8"))?
            } else {
                std::str::from_utf8(field)
                    .map_err(|_| invalid("JSONL field is not UTF-8"))?
            };
            write_json_string(&mut *row, text)?;
        }
    }
    if fields.next().is_some() {
        return Err(row_width_error());
    }
    row.extend_from_slice(b"}\n");
    output.write_all(row)
}

#[cfg(test)]
fn write_json_row<W: Write>(output: &mut W, headers: &[String], line: &[u8]) -> io::Result<()> {
    let escaped = headers
        .iter()
        .map(|header| {
            let mut key = Vec::new();
            write_json_string(&mut key, header)?;
            Ok(key)
        })
        .collect::<io::Result<Vec<_>>>()?;
    write_json_row_into(output, &escaped, line, &mut Vec::new(), &mut Vec::new())
}

pub struct JsonlWriter<W: Write> {
    output: W,
    headers: Option<Vec<Vec<u8>>>,
    line: Vec<u8>,
    encoded: Vec<u8>,
    decoded: Vec<u8>,
}

impl<W: Write> JsonlWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            headers: None,
            line: Vec::new(),
            encoded: Vec::new(),
            decoded: Vec::new(),
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        // Discard the physical CRLF separator across fragmented writes.
        if self.line.last() == Some(&b'\r') {
            self.line.pop();
        }
        if let Some(headers) = &self.headers {
            let result =
                write_json_row_into(&mut self.output, headers, &self.line, &mut self.encoded, &mut self.decoded);
            self.line.clear();
            return result;
        }
        let headers = parse_headers(&self.line);
        self.line.clear();
        self.headers = Some(
            headers?
                .into_iter()
                .map(|header| {
                    let mut encoded = Vec::with_capacity(header.len() + 2);
                    write_json_string(&mut encoded, &header)?;
                    Ok(encoded)
                })
                .collect::<io::Result<Vec<_>>>()?,
        );
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
            // Normalize CRLF only when LF completes the record. An escaped
            // TSV carriage return inside a field remains meaningful data.
            let fragment = if complete && self.line.is_empty() {
                fragment.strip_suffix(b"\r").unwrap_or(fragment)
            } else {
                fragment
            };
            if fragment.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
                return Err(invalid("JSONL row exceeds 1 MiB"));
            }
            // Complete data rows are already contiguous in the input.
            // Forward them into the reusable JSON encoder without copying.
            if complete && self.line.is_empty() {
                if let Some(headers) = self.headers.as_ref() {
                    write_json_row_into(&mut self.output, headers, fragment, &mut self.encoded, &mut self.decoded)?;
                    continue;
                }
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
    fn complete_data_rows_bypass_the_line_buffer() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        let before = writer.line.capacity();
        let value = "x".repeat(16 * 1024);
        writer.write_all(format!("{value}\n").as_bytes()).unwrap();
        assert_eq!(writer.line.capacity(), before);
        assert_eq!(
            writer.output,
            format!("{{\"value\":\"{value}\"}}\n").as_bytes()
        );
    }

    #[test]
    fn reuses_jsonl_response_buffer() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        let large = format!("{}\n", "a".repeat(4096));
        writer.write_all(large.as_bytes()).unwrap();
        let capacity = writer.encoded.capacity();
        writer.write_all(b"b\n").unwrap();
        assert_eq!(writer.encoded.capacity(), capacity);
    }

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
    fn missing_and_extra_fields_do_not_emit_partial_json() {
        let headers = vec!["id".to_owned(), "value".to_owned()];
        for invalid in [b"one".as_slice(), b"one\ttwo\tthree".as_slice()] {
            let mut output = Vec::new();
            assert_eq!(
                write_json_row(&mut output, &headers, invalid)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
            assert!(output.is_empty());
        }
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

    #[test]
    fn writer_reuses_escaped_json_keys_over_multiple_rows() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"name\"x\tvalue\n").unwrap();
        writer.write_all(b"alpha\t1\nbeta\t2\n").unwrap();
        assert_eq!(
            writer.output,
            b"{\"name\\\"x\":\"alpha\",\"value\":\"1\"}\n{\"name\\\"x\":\"beta\",\"value\":\"2\"}\n"
        );
    }
}

#[cfg(test)]
mod bounded_jsonl_header_set_tests {
    use super::*;

    #[test]
    fn preserves_header_order_and_uniqueness_at_small_large_boundary() {
        for count in [1, 2, 7, 8, 9, 16, 64] {
            let names = (0..count).map(|i| format!("col_{i}")).collect::<Vec<_>>();
            let input = names.join("\t");
            assert_eq!(parse_headers(input.as_bytes()).unwrap(), names);
            if count > 1 {
                let mut repeated = names.clone();
                repeated[count - 1] = names[0].clone();
                assert!(parse_headers(repeated.join("\t").as_bytes()).is_err());
            }
        }
    }

    #[test]
    fn rejects_duplicate_ninth_header_and_overwide_schema() {
        let names = (0..8).map(|i| format!("v{i}")).collect::<Vec<_>>();
        let repeated = format!("{}\tv0", names.join("\t"));
        assert!(parse_headers(repeated.as_bytes()).is_err());
        let oversized = (0..65)
            .map(|i| format!("x{i}"))
            .collect::<Vec<_>>()
            .join("\t");
        assert!(parse_headers(oversized.as_bytes()).is_err());
        assert!(parse_headers(b"v0\t\xff").is_err());
    }
}

#[cfg(test)]
mod crlf_record_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn contiguous_and_fragmented_crlf_lines_do_not_leak_carriage_returns() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"name\r\nalice\r\n").unwrap();
        writer.write_all(b"bo").unwrap();
        writer.write_all(b"b\r").unwrap();
        writer.write_all(b"\n").unwrap();
        assert_eq!(writer.output, "{\"name\":\"alice\"}\n{\"name\":\"bob\"}\n".as_bytes());
    }
}

#[cfg(test)]
mod reusable_tsv_decode_buffer_tests {
    use super::*;

    #[test]
    fn escaped_large_fields_reuse_buffer_and_preserve_json_escaping() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"message\n").unwrap();
        let wide = format!(r"{}\t{}", "a".repeat(8192), "b".repeat(8192));
        writer.write_all(format!("{wide}\n").as_bytes()).unwrap();
        let pointer = writer.decoded.as_ptr();
        let capacity = writer.decoded.capacity();
        assert!(capacity >= 16_385);
        writer.write_all(br"small\tvalue").unwrap();
        writer.write_all(b"\n").unwrap();
        assert_eq!(writer.decoded.as_ptr(), pointer);
        assert_eq!(writer.decoded.capacity(), capacity);
        assert!(writer.output.ends_with(b"{\"message\":\"small\\tvalue\"}\n"));
        assert!(writer.output.starts_with(b"{\"message\":\"aaaa"));
    }

    #[test]
    fn invalid_escaped_bytes_do_not_commit_a_partial_json_row() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"a\tb\nleft\tright\n").unwrap();
        let committed = writer.output.clone();
        assert!(writer.write_all(b"ok\tbad\\xFF").is_ok());
        assert!(writer.write_all(b"\n").is_err());
        assert_eq!(writer.output, committed);
    }
}
