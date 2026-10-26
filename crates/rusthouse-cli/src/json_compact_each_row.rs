//! JSONCompactEachRow-shaped output for the bounded SQL CLI.
//! Values stay strings (or JSON null) to preserve 128-bit integer precision.
use crate::jsonl::write_json_value;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

pub(super) struct JsonCompactEachRowWriter<W: Write> {
    output: W,
    columns: Option<usize>,
    line: Vec<u8>,
}

impl<W: Write> JsonCompactEachRowWriter<W> {
    pub(super) fn new(output: W) -> Self {
        Self {
            output,
            columns: None,
            line: Vec::new(),
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        if self.columns.is_none() {
            std::str::from_utf8(&self.line)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 header"))?;
            let width = 1 + self.line.iter().filter(|&&byte| byte == b'\t').count();
            if width > 128 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "too many columns"));
            }
            self.columns = Some(width);
            self.line.clear();
            return Ok(());
        }
        let width = 1 + self.line.iter().filter(|&&byte| byte == b'\t').count();
        if Some(width) != self.columns {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "JSON row width differs from header"));
        }
        self.output.write_all(b"[")?;
        for (index, value) in self.line.split(|byte| *byte == b'\t').enumerate() {
            if index != 0 {
                self.output.write_all(b",")?;
            }
            write_json_value(&mut self.output, value)?;
        }
        self.output.write_all(b"]\n")?;
        self.line.clear();
        Ok(())
    }

    pub(super) fn finish(mut self) -> io::Result<W> {
        if self.columns.is_none() || !self.line.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "incomplete JSON row stream"));
        }
        self.output.flush()?;
        Ok(self.output)
    }
}

impl<W: Write> Write for JsonCompactEachRowWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_line()?;
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "JSON row exceeds 1 MiB"));
                }
                self.line.push(byte);
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
    fn writes_compact_rows_with_escapes_nulls_and_duplicate_headers() {
        let mut writer = JsonCompactEachRowWriter::new(Vec::new());
        writer.write_all(b"number\tname\tname\n12\tgood\\tvalue\t\\N\n").unwrap();
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
}
