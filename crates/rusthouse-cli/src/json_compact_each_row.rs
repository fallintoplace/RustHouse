//! JSONCompactEachRow-shaped output for the bounded SQL CLI.
//! Values stay strings (or JSON null) to preserve 128-bit integer precision.
use crate::jsonl::write_json_value;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn count_tsv_fields(line: &[u8]) -> usize {
    1 + line.iter().filter(|&&byte| byte == b'\t').count()
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
        if let Some(columns) = self.columns {
            self.write_row(columns)?;
        } else {
            self.record_header()?;
        }
        self.line.clear();
        Ok(())
    }

    fn record_header(&mut self) -> io::Result<()> {
        std::str::from_utf8(&self.line)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 header"))?;
        let width = count_tsv_fields(&self.line);
        if width > 128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "too many columns",
            ));
        }
        self.columns = Some(width);
        Ok(())
    }

    fn write_row(&mut self, columns: usize) -> io::Result<()> {
        write_complete_row(&mut self.output, &mut self.encoded, &self.line, columns)
    }
}

fn write_complete_row<W: Write>(
    output: &mut W,
    encoded: &mut Vec<u8>,
    line: &[u8],
    columns: usize,
) -> io::Result<()> {
    // Stage one full JSON row and validate its width before sending it.
    // Counting fields in this encoding pass avoids a full input rescan.
    // Quotes and separators expand plain TSV rows by about three
    // bytes per field, so reserve that framing before serializing.
    let capacity = line
        .len()
        .saturating_add(columns.saturating_mul(3))
        .saturating_add(3);
    encoded.clear();
    encoded.reserve(capacity);
    encoded.write_all(b"[")?;
    let mut field_count = 0usize;
    for value in line.split(|byte| *byte == b'\t') {
        if field_count != 0 {
            encoded.write_all(b",")?;
        }
        write_json_value(&mut *encoded, value)?;
        field_count += 1;
    }
    if field_count != columns {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "JSON row width differs from header",
        ));
    }
    encoded.write_all(b"]\n")?;
    output.write_all(encoded)?;
    Ok(())
}

impl<W: Write> JsonCompactEachRowWriter<W> {
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
                    write_complete_row(&mut self.output, &mut self.encoded, fragment, columns)?;
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
    fn full_rows_bypass_input_staging_but_keep_output_framing() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        let before = writer.line.capacity();
        let value = "a".repeat(16 * 1024);
        writer.write_all(format!("{value}\n").as_bytes()).unwrap();
        assert_eq!(writer.line.capacity(), before);
        assert_eq!(
            writer.finish().unwrap(),
            format!("[\"{value}\"]\n").as_bytes()
        );
    }

    #[test]
    fn one_pass_encoding_rejects_wrong_column_counts_atomically() {
        let mut result = b"before".to_vec();
        let mut scratch = Vec::new();
        for (line, columns) in [
            (b"one".as_slice(), 2usize),
            (b"one\ttwo\tthree".as_slice(), 2),
            (b"one".as_slice(), 0),
        ] {
            assert!(write_complete_row(&mut result, &mut scratch, line, columns).is_err());
            assert_eq!(result, b"before");
        }
        write_complete_row(&mut result, &mut scratch, b"one\ttwo", 2).unwrap();
        assert_eq!(result, b"before[\"one\",\"two\"]\n");
    }

    #[test]
    fn reuses_output_allocation_across_shorter_rows() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer
            .write_all(format!("value\n{}\n", "a".repeat(4096)).as_bytes())
            .unwrap();
        let capacity = writer.encoded.capacity();
        writer.write_all(b"b\n").unwrap();
        assert_eq!(writer.encoded.capacity(), capacity);
    }

    #[test]
    fn writes_compact_rows_with_escapes_nulls_and_duplicate_headers() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer
            .write_all(b"number\tname\tname\n12\tgood\\tvalue\t\\N\n")
            .unwrap();
        writer.write_all(b"13\tcaf").unwrap();
        writer.write_all("é\tquoted\\x22word\n".as_bytes()).unwrap();
        assert_eq!(
            writer.finish().unwrap(),
            "[\"12\",\"good\\tvalue\",null]\n[\"13\",\"café\",\"quoted\\\"word\"]\n".as_bytes()
        );
    }

    #[test]
    fn empty_results_emit_nothing_and_incomplete_rows_fail() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        assert_eq!(writer.finish().unwrap(), b"");

        for bytes in [b"value".as_slice(), b"value\n1".as_slice()] {
            let mut writer = JsonCompactEachRowWriter::new(Vec::new());
            writer.write_all(bytes).unwrap();
            assert!(writer.finish().is_err());
        }
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer.write_all(b"a\tb\n1\n").unwrap_err();
    }
    #[test]
    fn rejects_invalid_later_field_without_partial_json_output() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer.write_all(b"first\tsecond\n").unwrap();
        assert_eq!(
            writer.write_all(b"good\tbad\\q\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(writer.output.is_empty());
    }

    #[test]
    fn processes_many_compact_rows_from_one_bulk_write() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        let input = format!("value\n{}", "42\n".repeat(1_000));
        writer.write_all(input.as_bytes()).unwrap();
        assert_eq!(
            writer.finish().unwrap(),
            "[\"42\"]\n".repeat(1_000).as_bytes()
        );
    }

    #[test]
    fn compact_output_keeps_plain_row_field_boundaries() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer.write_all(b"a\tb\tc\n1\t2\t3\n").unwrap();
        assert_eq!(writer.finish().unwrap(), b"[\"1\",\"2\",\"3\"]\n");
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
