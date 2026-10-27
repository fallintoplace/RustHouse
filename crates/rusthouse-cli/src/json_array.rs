use crate::jsonl::JsonlWriter;
use std::io::{self, Write};

const MAX_JSON_ROW_BYTES: usize = 4 * 1024 * 1024;

fn write_indent<W: Write>(output: &mut W, depth: usize) -> io::Result<()> {
    output.write_all(b"\n")?;
    for _ in 0..=depth {
        output.write_all(b"  ")?;
    }
    Ok(())
}

/// Indent validated JSONL rows, keeping quotes and escaped syntax intact.
fn write_pretty_row<W: Write>(output: &mut W, row: &[u8]) -> io::Result<()> {
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, &byte) in row.iter().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => {
                output.write_all(&row[start..=index])?;
                depth += 1;
                write_indent(output, depth)?;
                start = index + 1;
            }
            b'}' | b']' => {
                output.write_all(&row[start..index])?;
                depth = depth.saturating_sub(1);
                write_indent(output, depth)?;
                output.write_all(&row[index..=index])?;
                start = index + 1;
            }
            b',' => {
                output.write_all(&row[start..=index])?;
                write_indent(output, depth)?;
                start = index + 1;
            }
            b':' => {
                output.write_all(&row[start..=index])?;
                output.write_all(b" ")?;
                start = index + 1;
            }
            _ => {}
        }
    }
    output.write_all(&row[start..])
}

/// Convert a stream of JSONL objects to a valid JSON array without buffering all rows.
struct JsonArraySink<W: Write> {
    output: W,
    row: Vec<u8>,
    first: bool,
    pretty: bool,
}

impl<W: Write> JsonArraySink<W> {
    fn new(output: W, pretty: bool) -> Self {
        Self {
            output,
            row: Vec::new(),
            first: true,
            pretty,
        }
    }

    fn finish_row(&mut self) -> io::Result<()> {
        if self.first {
            self.output
                .write_all(if self.pretty { b"[\n  " } else { b"[" })?;
        } else {
            self.output
                .write_all(if self.pretty { b",\n  " } else { b"," })?;
        }
        if self.pretty {
            write_pretty_row(&mut self.output, &self.row)?;
        } else {
            self.output.write_all(&self.row)?;
        }
        self.row.clear();
        self.first = false;
        Ok(())
    }

    fn finish(mut self) -> io::Result<W> {
        if !self.row.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete JSON row",
            ));
        }
        if self.first {
            self.output.write_all(b"[]\n")?;
        } else if self.pretty {
            self.output.write_all(b"\n]\n")?;
        } else {
            self.output.write_all(b"]\n")?;
        }
        self.output.flush()?;
        Ok(self.output)
    }
}

impl<W: Write> Write for JsonArraySink<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for chunk in bytes.split_inclusive(|&byte| byte == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            let fragment = if complete {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            if fragment.len() > MAX_JSON_ROW_BYTES.saturating_sub(self.row.len()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "JSON object exceeds 4 MiB",
                ));
            }
            self.row.extend_from_slice(fragment);
            if complete {
                self.finish_row()?;
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

/// Use JsonlWriter's validated, escaped row format for a streaming JSON array.
pub struct JsonArrayWriter<W: Write> {
    inner: JsonlWriter<JsonArraySink<W>>,
    incomplete_line: bool,
}

impl<W: Write> JsonArrayWriter<W> {
    pub fn new(output: W) -> Self {
        Self::with_style(output, false)
    }

    pub fn new_pretty(output: W) -> Self {
        Self::with_style(output, true)
    }

    fn with_style(output: W, pretty: bool) -> Self {
        Self {
            inner: JsonlWriter::new(JsonArraySink::new(output, pretty)),
            incomplete_line: false,
        }
    }

    /// Finalization is explicit because JSON output needs a closing bracket.
    pub fn finish(mut self) -> io::Result<W> {
        if self.incomplete_line {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete JSON input line",
            ));
        }
        self.inner.flush()?;
        self.inner.into_inner().finish()
    }
}

impl<W: Write> Write for JsonArrayWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        if let Some(last) = bytes.last() {
            self.incomplete_line = *last != b'\n';
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_streamed_json_array_with_escaped_values() {
        let mut writer = JsonArrayWriter::new(Vec::new());
        writer.write_all(b"number\tname\n12\t").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"\"x\"\n13\tNULL\n").unwrap();
        let output = writer.finish().unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "[{\"number\":\"12\",\"name\":\"\\\"x\\\"\"},{\"number\":\"13\",\"name\":null}]\n"
        );
    }

    #[test]
    fn pretty_prints_multiple_objects_and_preserves_escaped_strings() {
        let mut writer = JsonArrayWriter::new_pretty(Vec::new());
        writer.write_all(b"name\tnullable\n").unwrap();
        writer.write_all(b"hello\\nworld\t\\N\n").unwrap();
        writer.write_all(b"\"quoted\"\tvalue\n").unwrap();
        assert_eq!(
            String::from_utf8(writer.finish().unwrap()).unwrap(),
            "[\n  {\n    \"name\": \"hello\\nworld\",\n    \"nullable\": null\n  },\n  {\n    \"name\": \"\\\"quoted\\\"\",\n    \"nullable\": \"value\"\n  }\n]\n"
        );
        let mut empty = JsonArrayWriter::new_pretty(Vec::new());
        empty.write_all(b"id\n").unwrap();
        assert_eq!(empty.finish().unwrap(), b"[]\n");
    }

    #[test]
    fn empty_result_and_invalid_rows() {
        let mut writer = JsonArrayWriter::new(Vec::new());
        writer.write_all(b"number\n").unwrap();
        assert_eq!(writer.finish().unwrap(), b"[]\n");
        let mut writer = JsonArrayWriter::new(Vec::new());
        assert!(writer.write_all(b"x\tx\n").is_err());
    }

    #[test]
    fn rejects_unterminated_headers_and_rows_at_finish() {
        for input in [b"number".as_slice(), b"number\n1".as_slice()] {
            let mut writer = JsonArrayWriter::new(Vec::new());
            writer.write_all(input).unwrap();
            assert_eq!(
                writer.finish().unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }

        let mut writer = JsonArrayWriter::new(Vec::new());
        writer.write_all(b"number\n1").unwrap();
        writer.write_all(b"\n").unwrap();
        assert_eq!(writer.finish().unwrap(), b"[{\"number\":\"1\"}]\n");
    }
    #[test]
    fn array_sink_bulk_copies_large_json_objects() {
        let large_row = format!("{{\"value\":\"{}\"}}", "x".repeat(64 * 1024));
        let mut sink = JsonArraySink::new(Vec::new(), false);
        sink.write_all(large_row.as_bytes()).unwrap();
        sink.write_all(b"\n{}\n").unwrap();
        assert_eq!(
            sink.finish().unwrap(),
            format!("[{large_row},{{}}]\n").as_bytes()
        );
    }
}
