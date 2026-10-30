//! Write one JSON array per bounded TSV result row.
use super::{jsonl::decode_tsv_value, write_json_string};
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn write_compact_row_into<W: Write>(
    output: &mut W,
    line: &[u8],
    columns: usize,
    row: &mut Vec<u8>,
) -> io::Result<()> {
    let width = 1 + line.iter().filter(|&&byte| byte == b'\t').count();
    if width != columns {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "JSON row width differs from header",
        ));
    }
    // Reserve framing for common unescaped fields before writing quotes.
    row.clear();
    row.reserve(
        line.len()
            .saturating_add(columns.saturating_mul(3))
            .saturating_add(3),
    );
    row.write_all(b"[")?;
    for (index, value) in line.split(|byte| *byte == b'\t').enumerate() {
        if index > 0 {
            row.write_all(b",")?;
        }
        if value == b"\\N" {
            row.write_all(b"null")?;
        } else {
            if value.contains(&b'\\') {
                write_json_string(&mut *row, &decode_tsv_value(value)?)?;
            } else {
                let text = std::str::from_utf8(value).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 field")
                })?;
                write_json_string(&mut *row, text)?;
            }
        }
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
            if fragment.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "JSON row exceeds 1 MiB",
                ));
            }
            if complete && self.line.is_empty() {
                if let Some(columns) = self.columns {
                    write_compact_row_into(
                        &mut self.output,
                        fragment,
                        columns,
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
