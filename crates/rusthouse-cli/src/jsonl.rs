use crate::tsv_escape::decode_tsv_field;
use std::collections::HashSet;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_JSONL_COLUMNS: usize = 64;

/// Convert tab-separated numeric query output to one JSON object per row.
/// Values are strings to preserve UInt128/Int128 precision. NULL becomes JSON null.
pub struct JsonlWriter<W: Write> {
    output: W,
    headers: Option<Vec<Vec<u8>>>,
    line: Vec<u8>,
    encoded: Vec<u8>,
}

impl<W: Write> JsonlWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            headers: None,
            line: Vec::new(),
            encoded: Vec::new(),
        }
    }

    pub fn into_inner(self) -> W {
        self.output
    }

    fn finish_line(&mut self) -> io::Result<()> {
        if self.headers.is_none() {
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
            return Ok(());
        }

        let headers = self.headers.as_ref().unwrap();
        let result = write_json_row_into(&mut self.output, headers, &self.line, &mut self.encoded);
        self.line.clear();
        result
    }
}

fn parse_headers(line: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let mut seen = HashSet::new();
    let mut headers = Vec::new();
    for header in line.split(|byte| *byte == b'\t') {
        if headers.len() >= MAX_JSONL_COLUMNS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSONL supports at most 64 columns",
            ));
        }
        if header.is_empty() || std::str::from_utf8(header).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSONL header is empty or not UTF-8",
            ));
        }
        if !seen.insert(header) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSONL requires unique projected column names",
            ));
        }
        headers.push(header.to_vec());
    }
    Ok(headers)
}

fn jsonl_row_width_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "JSONL row does not match header column count")
}

fn write_json_row_into<W: Write>(
    output: &mut W,
    headers: &[Vec<u8>],
    line: &[u8],
    row: &mut Vec<u8>,
) -> io::Result<()> {
    // Visit each TSV field once. Validate the shape in the local row buffer
    // so malformed records never partially write to the output stream.
    let mut fields = line.split(|byte| *byte == b'\t');
    row.clear();
    row.reserve(line.len() + headers.len() * 8 + 3);
    row.write_all(b"{")?;
    for (index, header) in headers.iter().enumerate() {
        let value = fields.next().ok_or_else(jsonl_row_width_error)?;
        if index > 0 {
            row.write_all(b",")?;
        }
        row.write_all(header)?;
        row.write_all(b":")?;
        write_json_value(&mut *row, value)?;
    }
    if fields.next().is_some() {
        return Err(jsonl_row_width_error());
    }
    row.write_all(b"}\n")?;
    output.write_all(row)
}

#[cfg(test)]
fn write_json_row<W: Write>(output: &mut W, headers: &[Vec<u8>], line: &[u8]) -> io::Result<()> {
    let encoded = headers
        .iter()
        .map(|header| {
            let mut escaped = Vec::with_capacity(header.len() + 2);
            write_json_string(&mut escaped, header)?;
            Ok(escaped)
        })
        .collect::<io::Result<Vec<_>>>()?;
    write_json_row_into(output, &encoded, line, &mut Vec::new())
}

// Keep NULL marker handling in one place for every JSONL field.
pub(super) fn write_json_value<W: Write>(output: &mut W, value: &[u8]) -> io::Result<()> {
    if value == b"\\N" {
        output.write_all(b"null")
    } else {
        if value.contains(&b'\\') {
            let decoded = decode_tsv_field(value)?;
            write_json_string(output, &decoded)
        } else {
            write_json_string(output, value)
        }
    }
}

fn write_json_string<W: Write>(writer: &mut W, input: &[u8]) -> io::Result<()> {
    let input = std::str::from_utf8(input).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "JSONL field is not valid UTF-8")
    })?;
    let bytes = input.as_bytes();
    if bytes.len() <= 256
        && bytes
            .iter()
            .all(|&byte| (b' '..=b'~').contains(&byte) && byte != b'"' && byte != b'\\')
    {
        if bytes.len() <= 64 {
            let mut quoted = [0u8; 66];
            quoted[0] = b'"';
            quoted[1..bytes.len() + 1].copy_from_slice(bytes);
            quoted[bytes.len() + 1] = b'"';
            return writer.write_all(&quoted[..bytes.len() + 2]);
        }
        let mut quoted = [0u8; 258];
        quoted[0] = b'"';
        quoted[1..bytes.len() + 1].copy_from_slice(bytes);
        quoted[bytes.len() + 1] = b'"';
        return writer.write_all(&quoted[..bytes.len() + 2]);
    }
    // Escaping every quote using separate writes becomes expensive for wide
    // JSONL exports. Keep the existing Unicode-control escaping semantics.
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut buffer = [0u8; 4096];
    buffer[0] = b'"';
    let mut used = 1;
    for ch in input.chars() {
        let mut codepoint_bytes = [0u8; 4];
        let mut unicode_escape = [0u8; 6];
        let encoded: &[u8] = match ch {
            '"' => br#"\""#,
            '\\' => br"\\",
            '\n' => br"\n",
            '\r' => br"\r",
            '\t' => br"\t",
            ch if ch.is_control() => {
                let value = ch as u32;
                unicode_escape = [
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX[((value >> 4) & 15) as usize],
                    HEX[(value & 15) as usize],
                ];
                &unicode_escape
            }
            ch => ch.encode_utf8(&mut codepoint_bytes).as_bytes(),
        };
        if used + encoded.len() > buffer.len() {
            writer.write_all(&buffer[..used])?;
            used = 0;
        }
        buffer[used..used + encoded.len()].copy_from_slice(encoded);
        used += encoded.len();
    }
    if used == buffer.len() {
        writer.write_all(&buffer)?;
        used = 0;
    }
    buffer[used] = b'"';
    writer.write_all(&buffer[..used + 1])
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
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "JSONL row exceeds 1 MiB",
                ));
            }
            if complete && self.line.is_empty() {
                if let Some(headers) = self.headers.as_ref() {
                    // Complete data rows can be encoded straight from this write.
                    write_json_row_into(
                        &mut self.output,
                        headers,
                        fragment,
                        &mut self.encoded,
                    )?;
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
    fn complete_large_rows_do_not_stage_the_input() {
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
    fn reuses_jsonl_output_allocation_for_shorter_rows() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        let large = format!("{}\n", "a".repeat(4096));
        writer.write_all(large.as_bytes()).unwrap();
        let capacity = writer.encoded.capacity();
        writer.write_all(b"b\n").unwrap();
        assert_eq!(writer.encoded.capacity(), capacity);
        assert!(writer.output.ends_with(b"{\"value\":\"b\"}\n"));
    }

    #[test]
    fn writes_short_ascii_json_strings_once() {
        struct Calls {
            writes: usize,
            output: Vec<u8>,
        }
        impl Write for Calls {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.output.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut sink = Calls {
            writes: 0,
            output: Vec::new(),
        };
        write_json_string(&mut sink, b"hello world").unwrap();
        assert_eq!(sink.writes, 1);
        assert_eq!(sink.output, br#""hello world""#);
    }

    #[test]
    fn writes_plain_json_fields_in_bulk_without_losing_unicode() {
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
        write_json_string(&mut output, "plain café".as_bytes()).unwrap();
        assert_eq!(output.calls, 3);
        assert_eq!(output.bytes, "\"plain café\"".as_bytes());
    }

    #[test]
    fn writes_escaped_json_and_preserves_large_values() {
        let mut output = Vec::new();
        {
            let mut writer = JsonlWriter::new(&mut output);
            writer
                .write_all(b"name\tamount\n\"a\"\t340282366920938463463374607431768211455\n")
                .unwrap();
            writer.write_all(b"missing\t\\N\n").unwrap();
            writer.flush().unwrap();
        }
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "{\"name\":\"\\\"a\\\"\",\"amount\":\"340282366920938463463374607431768211455\"}\n{\"name\":\"missing\",\"amount\":null}\n"
        );
    }

    #[test]
    fn rejects_duplicate_headers_and_incomplete_rows() {
        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(b"a\ta\n").is_err());

        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"a\tb\n").unwrap();
        assert!(writer.write_all(b"1\n").is_err());
    }

    #[test]
    fn rejects_empty_and_excessive_headers() {
        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(b"\n").is_err());

        let columns = (0..65)
            .map(|index| format!("c{index}"))
            .collect::<Vec<_>>()
            .join("\t");
        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(format!("{columns}\n").as_bytes()).is_err());
    }

    #[test]
    fn rejects_oversized_rows() {
        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(&vec![b'a'; MAX_LINE_BYTES + 1]).is_err());
    }

    #[test]
    fn accepts_exact_row_limit_across_chunk_boundaries() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        let large = vec![b'a'; MAX_LINE_BYTES];
        writer.write_all(&large[..MAX_LINE_BYTES / 2]).unwrap();
        writer.write_all(&large[MAX_LINE_BYTES / 2..]).unwrap();
        writer.write_all(b"\n").unwrap();
        assert!(writer.output.starts_with(b"{\"value\":\""));
        assert!(writer.output.ends_with(b"\"}\n"));

        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        writer.write_all(&large).unwrap();
        assert_eq!(
            writer.write_all(b"a").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn handles_multiple_complete_rows_and_fragmented_tail() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"value\none\ntw").unwrap();
        writer.write_all(b"o\nthree\n").unwrap();
        assert_eq!(
            writer.output,
            b"{\"value\":\"one\"}\n{\"value\":\"two\"}\n{\"value\":\"three\"}\n"
        );
    }

    #[test]
    fn flush_does_not_split_headers_or_rows() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"name\tamount").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"\nabc\t1").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"2\n").unwrap();
        assert_eq!(
            String::from_utf8(writer.output).unwrap(),
            "{\"name\":\"abc\",\"amount\":\"12\"}\n"
        );
    }

    #[test]
    fn invalid_late_field_does_not_write_partial_json() {
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
    fn rejects_invalid_utf8_in_headers_and_values() {
        let mut writer = JsonlWriter::new(Vec::new());
        assert_eq!(
            writer.write_all(b"\xff\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"name\n").unwrap();
        assert_eq!(
            writer.write_all(b"\xff\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[test]
    fn separates_null_markers_from_literal_text() {
        let mut output = Vec::new();
        for (index, value) in [b"NULL".as_slice(), b"\\N", b"null", b"NULLABLE"]
            .into_iter()
            .enumerate()
        {
            if index > 0 {
                output.push(b',');
            }
            write_json_value(&mut output, value).unwrap();
        }
        assert_eq!(output, b"\"NULL\",null,\"null\",\"NULLABLE\"");
    }

    #[test]
    fn keeps_literal_null_text_distinct_from_typed_nulls() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"value\nNULL\n\\N\n").unwrap();
        assert_eq!(
            writer.into_inner(),
            b"{\"value\":\"NULL\"}\n{\"value\":null}\n"
        );
    }

    #[test]
    fn renders_typed_null_markers_as_json_null() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"message\tamount\n\\N\t1\n").unwrap();
        assert_eq!(
            writer.into_inner(),
            b"{\"message\":null,\"amount\":\"1\"}\n"
        );
    }

    #[test]
    fn formats_wide_rows_without_cloning_each_field() {
        let header = (0..32)
            .map(|index| format!("field{index}"))
            .collect::<Vec<_>>()
            .join("\t");
        let row = (0..32)
            .map(|index| format!("value{index}"))
            .collect::<Vec<_>>()
            .join("\t");
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(header.as_bytes()).unwrap();
        writer.write_all(b"\n").unwrap();
        for _ in 0..2 {
            writer.write_all(row.as_bytes()).unwrap();
            writer.write_all(b"\n").unwrap();
        }
        let output = String::from_utf8(writer.into_inner()).unwrap();
        assert_eq!(output.lines().count(), 2);
        assert_eq!(output.matches("\"field31\":\"value31\"").count(), 2);
        assert!(output.starts_with("{\"field0\":\"value0\","));
    }
}

#[cfg(test)]
mod decoded_tsv_json_regressions {
    use super::*;

    #[test]
    fn restores_escaped_sql_strings_before_json_quoting() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer
            .write_all(b"value\nline\\nnext\\tcell\\\\end\n")
            .unwrap();
        assert_eq!(
            writer.into_inner(),
            b"{\"value\":\"line\\nnext\\tcell\\\\end\"}\n"
        );
    }

    #[test]
    fn rejects_bad_tsv_escapes_instead_of_exporting_changed_data() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        assert_eq!(
            writer.write_all(b"bad\\q\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[test]
    fn skips_decode_for_plain_fields_without_changing_output() {
        let mut output = Vec::new();
        write_json_value(&mut output, "café".as_bytes()).unwrap();
        write_json_value(&mut output, b"a\\tb").unwrap();
        assert_eq!(output, "\"café\"\"a\\tb\"".as_bytes());
        assert!(write_json_value(&mut Vec::new(), &[0xff]).is_err());
    }
}

#[cfg(test)]
mod single_pass_row_tests {
    use super::*;

    #[test]
    fn mismatched_fields_never_emit_partial_json() {
        let headers = vec![b"id".to_vec(), b"value".to_vec()];
        for malformed in [b"one".as_slice(), b"one\ttwo\tthree".as_slice()] {
            let mut output = Vec::new();
            assert_eq!(
                write_json_row(&mut output, &headers, malformed)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
            assert!(output.is_empty());
        }
        let mut output = Vec::new();
        write_json_row(&mut output, &headers, b"one\ttwo").unwrap();
        assert_eq!(output, b"{\"id\":\"one\",\"value\":\"two\"}\n");
    }

    #[test]
    fn medium_json_fields_use_one_output_write() {
        struct CountingWriter {
            calls: usize,
            encoded: Vec<u8>,
        }

        impl Write for CountingWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.encoded.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut writer = CountingWriter {
            calls: 0,
            encoded: Vec::new(),
        };
        let input = "x".repeat(200);
        write_json_string(&mut writer, input.as_bytes()).unwrap();
        assert_eq!(writer.calls, 1);
        assert_eq!(writer.encoded.len(), 202);
        assert_eq!(writer.encoded[0], b'"');
        assert_eq!(writer.encoded.last(), Some(&b'"'));
    }

    #[test]
    fn jsonl_writer_caches_escaped_header_across_rows() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"key\"name\n").unwrap();
        writer.write_all(b"first\nsecond\n").unwrap();
        assert_eq!(
            writer.output,
            b"{\"key\\\"name\":\"first\"}\n{\"key\\\"name\":\"second\"}\n"
        );
    }

}

#[cfg(test)]
mod batched_jsonl_escaping_tests {
    use super::*;

    #[test]
    fn preserves_unicode_and_escapes_non_ascii_controls() {
        let mut output = Vec::new();
        write_json_string(&mut output, "café\n\t\u{0085}\\\"".as_bytes()).unwrap();
        assert_eq!(output, "\"café\\n\\t\\u0085\\\\\\\"\"".as_bytes());
    }

    #[test]
    fn quote_heavy_jsonl_field_uses_few_write_calls() {
        struct Count {
            writes: usize,
            output: Vec<u8>,
        }
        impl Write for Count {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.output.extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let input = "\"".repeat(10_000);
        let mut output = Count {
            writes: 0,
            output: Vec::new(),
        };
        write_json_string(&mut output, input.as_bytes()).unwrap();
        assert_eq!(output.output.len(), 20_002);
        assert!(output.writes <= 6);
    }
}
