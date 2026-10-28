//! JSONCompactEachRow-shaped output for the bounded SQL CLI.
//! Values stay strings (or JSON null) to preserve 128-bit integer precision.
use crate::jsonl::write_json_value;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn field_count(line: &[u8]) -> usize {
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
        let width = field_count(&self.line);
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
        if field_count(&self.line) != columns {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSON row width differs from header",
            ));
        }
        // Quotes and separators expand plain TSV rows by about three
        // bytes per field, so reserve that framing before serializing.
        let capacity = self
            .line
            .len()
            .saturating_add(columns.saturating_mul(3))
            .saturating_add(3);
        self.encoded.clear();
        self.encoded.reserve(capacity);
        let encoded = &mut self.encoded;
        encoded.write_all(b"[")?;
        for (index, value) in self.line.split(|byte| *byte == b'\t').enumerate() {
            if index != 0 {
                encoded.write_all(b",")?;
            }
            write_json_value(&mut *encoded, value)?;
        }
        encoded.write_all(b"]\n")?;
        self.output.write_all(encoded)?;
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
            if fragment.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "JSON row exceeds 1 MiB",
                ));
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
