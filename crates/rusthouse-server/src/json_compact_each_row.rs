//! Write one JSON array per bounded TSV result row.
use super::{jsonl::decode_tsv_value, write_json_string};
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn write_compact_field(output: &mut Vec<u8>, value: &[u8]) -> io::Result<()> {
    if value == b"\\N" {
        output.write_all(b"null")?;
    } else if value.contains(&b'\\') {
        write_json_string(output, &decode_tsv_value(value)?)?;
    } else if value
        .iter()
        .all(|&byte| (0x20..0x7f).contains(&byte) && byte != b'"')
    {
        // A common TSV field is already printable ASCII without a JSON
        // escape. Append quotes and payload directly to the row buffer.
        output.push(b'"');
        output.extend_from_slice(value);
        output.push(b'"');
    } else {
        let text = std::str::from_utf8(value)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 field"))?;
        write_json_string(output, text)?;
    }
    Ok(())
}

fn write_compact_row_into<W: Write>(
    output: &mut W,
    line: &[u8],
    columns: usize,
    row: &mut Vec<u8>,
) -> io::Result<()> {
    // The private staging buffer keeps malformed rows out of the output.
    // Validate the column count while encoding to avoid rescanning the input.
    // Reserve framing for common unescaped fields before writing quotes.
    row.clear();
    row.reserve(
        line.len()
            .saturating_add(columns.saturating_mul(3))
            .saturating_add(3),
    );
    row.write_all(b"[")?;
    let mut width = 0;
    for value in line.split(|byte| *byte == b'\t') {
        if width >= columns {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSON row width differs from header",
            ));
        }
        if width > 0 {
            row.write_all(b",")?;
        }
        width += 1;
        write_compact_field(row, value)?;
    }
    if width != columns {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "JSON row width differs from header",
        ));
    }
    row.write_all(b"]\n")?;
    output.write_all(row)
}

#[cfg(test)]
fn write_compact_row<W: Write>(output: &mut W, line: &[u8], columns: usize) -> io::Result<()> {
    write_compact_row_into(output, line, columns, &mut Vec::new())
}

pub(super) struct JsonCompactEachRowWriter<W: Write> {
    output: W,
    columns: Option<usize>,
    line: Vec<u8>,
    encoded: Vec<u8>,
}

impl<W: Write> JsonCompactEachRowWriter<W> {
    pub(super) fn new(output: W) -> Self {
        Self {
            output,
            columns: None,
            line: Vec::new(),
            encoded: Vec::new(),
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        // Discard the physical CRLF separator across fragmented writes.
        if self.line.last() == Some(&b'\r') {
            self.line.pop();
        }
        let Some(columns) = self.columns else {
            std::str::from_utf8(&self.line)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid TSV header"))?;
            let columns = 1 + self.line.iter().filter(|&&byte| byte == b'\t').count();
            if columns > 128 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "too many columns",
                ));
            }
            self.columns = Some(columns);
            self.line.clear();
            return Ok(());
        };
        write_compact_row_into(&mut self.output, &self.line, columns, &mut self.encoded)?;
        self.line.clear();
        Ok(())
    }

    pub(super) fn finish(mut self) -> io::Result<W> {
        if self.columns.is_none() || !self.line.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete JSON row stream",
            ));
        }
        self.output.flush()?;
        Ok(self.output)
    }
}

impl<W: Write> Write for JsonCompactEachRowWriter<W> {
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
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "JSON row exceeds 1 MiB",
                ));
            }
            if complete && self.line.is_empty() {
                if let Some(columns) = self.columns {
                    write_compact_row_into(&mut self.output, fragment, columns, &mut self.encoded)?;
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
    fn complete_large_rows_do_not_allocate_a_line_buffer() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        let before = writer.line.capacity();
        let value = "z".repeat(16 * 1024);
        writer.write_all(format!("{value}\n").as_bytes()).unwrap();
        assert_eq!(writer.line.capacity(), before);
        assert_eq!(
            writer.finish().unwrap(),
            format!("[\"{value}\"]\n").as_bytes()
        );
    }

    #[test]
    fn response_buffer_is_reused_for_shorter_rows() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer
            .write_all(format!("value\n{}\n", "a".repeat(4096)).as_bytes())
            .unwrap();
        let capacity = writer.encoded.capacity();
        writer.write_all(b"b\n").unwrap();
        assert_eq!(writer.encoded.capacity(), capacity);
    }

    #[test]
    fn processes_many_rows_from_one_bulk_write() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        let input = format!("value\n{}", "42\n".repeat(1_000));
        writer.write_all(input.as_bytes()).unwrap();
        assert_eq!(
            writer.finish().unwrap(),
            "[\"42\"]\n".repeat(1_000).as_bytes()
        );
    }

    #[test]
    fn streams_arrays_without_headers_and_preserves_textual_nulls() {
        let mut out = JsonCompactEachRowWriter::new(Vec::new());
        out.write_all(b"id\tlabel\tmissing\n7\thello\\tworld\t\\N\n")
            .unwrap();
        out.write_all(b"8\tNULL\t\\").unwrap();
        out.write_all(b"N\n").unwrap();
        assert_eq!(
            out.finish().unwrap(),
            b"[\"7\",\"hello\\tworld\",null]\n[\"8\",\"NULL\",null]\n"
        );
    }

    #[test]
    fn invalid_late_field_does_not_emit_a_partial_json_row() {
        let mut output = Vec::new();
        assert!(write_compact_row(&mut output, b"ok\tbad\\q", 2).is_err());
        assert!(output.is_empty());
    }

    #[test]
    fn one_pass_width_validation_does_not_emit_failed_rows() {
        let mut output = b"prefix".to_vec();
        for (line, columns) in [
            (b"a\tb".as_slice(), 3usize),
            (b"a\tb\tc".as_slice(), 2),
            (b"a\tb".as_slice(), 0),
        ] {
            assert!(write_compact_row(&mut output, line, columns).is_err());
            assert_eq!(output, b"prefix");
        }
        write_compact_row(&mut output, b"a\tb", 2).unwrap();
        assert_eq!(output, b"prefix[\"a\",\"b\"]\n");
    }

    #[test]
    fn compact_row_validates_width_before_writing() {
        let mut output = Vec::new();
        assert!(write_compact_row(&mut output, b"only-one", 2).is_err());
        assert!(output.is_empty());
        write_compact_row(&mut output, b"1\t\\N", 2).unwrap();
        assert_eq!(output, b"[\"1\",null]\n");
    }

    #[test]
    fn rejects_truncated_bad_width_and_invalid_escapes() {
        let mut out = JsonCompactEachRowWriter::new(Vec::new());
        out.write_all(b"id\n").unwrap();
        assert_eq!(out.finish().unwrap(), b"");

        let mut out = JsonCompactEachRowWriter::new(Vec::new());
        out.write_all(b"id\tlabel\n7\n").unwrap_err();

        let mut out = JsonCompactEachRowWriter::new(Vec::new());
        out.write_all(b"id\n7").unwrap();
        assert!(out.finish().is_err());

        let mut out = JsonCompactEachRowWriter::new(Vec::new());
        out.write_all(b"id\nbad\\q\n").unwrap_err();
    }
    #[test]
    fn borrows_utf8_and_still_decodes_tsv_escapes() {
        let mut output = Vec::new();
        write_compact_row(&mut output, "1\tcafé".as_bytes(), 2).unwrap();
        write_compact_row(&mut output, b"2\ta\\tb", 2).unwrap();
        assert_eq!(output, "[\"1\",\"café\"]\n[\"2\",\"a\\tb\"]\n".as_bytes());
        assert!(write_compact_row(&mut Vec::new(), &[0xff], 1).is_err());
    }

    #[test]
    fn plain_multi_column_rows_fit_the_reserved_framing() {
        let mut output = Vec::new();
        write_compact_row(&mut output, b"a\tb\tc", 3).unwrap();
        assert_eq!(output, b"[\"a\",\"b\",\"c\"]\n");
    }
}

#[cfg(test)]
mod overwide_row_tests {
    use super::*;

    #[test]
    fn overwide_compact_rows_leave_the_output_untouched() {
        let mut output = Vec::new();
        let mut row = Vec::new();
        let error = write_compact_row_into(&mut output, b"one\ttwo", 1, &mut row).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(output.is_empty());
        write_compact_row_into(&mut output, b"one", 1, &mut row).unwrap();
        assert_eq!(output, b"[\"one\"]\n");
    }
}

#[cfg(test)]
mod safe_ascii_compact_field_tests {
    use super::*;

    #[test]
    fn long_unescaped_ascii_field_is_framed_without_reencoding() {
        let value = b"ABC-_0123456789+".repeat(4096);
        let mut output = Vec::new();
        write_compact_field(&mut output, &value).unwrap();
        assert_eq!(output.len(), value.len() + 2);
        assert_eq!(&output[1..output.len() - 1], value.as_slice());
        assert_eq!(output[0], b'"');
        assert_eq!(*output.last().unwrap(), b'"');
    }

    #[test]
    fn unsafe_ascii_unicode_null_and_invalid_utf8_keep_existing_rules() {
        for source in [
            b"\"quote\"".as_slice(),
            b"back\\slash".as_slice(),
            b"".as_slice(),
            b"caf\xc3\xa9".as_slice(),
            b"line\x01break".as_slice(),
        ] {
            let mut actual = Vec::new();
            write_compact_field(&mut actual, source).unwrap();
            let mut expected = Vec::new();
            let text = if source.contains(&b'\\') {
                decode_tsv_value(source).unwrap()
            } else {
                std::str::from_utf8(source).unwrap().to_owned()
            };
            write_json_string(&mut expected, &text).unwrap();
            assert_eq!(actual, expected);
        }
        let mut null = Vec::new();
        write_compact_field(&mut null, b"\\N").unwrap();
        assert_eq!(null, b"null");
        assert!(write_compact_field(&mut Vec::new(), &[0xff, 0xfe]).is_err());
    }
}

#[cfg(test)]
mod crlf_record_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn contiguous_and_fragmented_crlf_lines_do_not_leak_carriage_returns() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer.write_all(b"name\r\nalice\r\n").unwrap();
        writer.write_all(b"bo").unwrap();
        writer.write_all(b"b\r").unwrap();
        writer.write_all(b"\n").unwrap();
        assert_eq!(writer.output, "[\"alice\"]\n[\"bob\"]\n".as_bytes());
    }
}
