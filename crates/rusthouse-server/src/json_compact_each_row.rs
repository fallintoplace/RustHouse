//! Write one JSON array per bounded TSV result row.
use super::{jsonl::decode_tsv_value, write_json_string};
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn write_compact_row<W: Write>(output: &mut W, line: &[u8], columns: usize) -> io::Result<()> {
    let width = 1 + line.iter().filter(|&&byte| byte == b'\t').count();
    if width != columns {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "JSON row width differs from header",
        ));
    }
    output.write_all(b"[")?;
    for (index, value) in line.split(|byte| *byte == b'\t').enumerate() {
        if index > 0 {
            output.write_all(b",")?;
        }
        if value == b"\\N" {
            output.write_all(b"null")?;
        } else {
            write_json_string(output, &decode_tsv_value(value)?)?;
        }
    }
    output.write_all(b"]\n")
}

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
        write_compact_row(&mut self.output, &self.line, columns)?;
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
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_line()?;
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "JSON row exceeds 1 MiB",
                    ));
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
}
