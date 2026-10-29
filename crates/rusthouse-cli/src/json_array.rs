use crate::jsonl::JsonlWriter;
use rusthouse_format::write_pretty_json_row;
use std::io::{self, Write};

const MAX_JSON_ROW_BYTES: usize = 4 * 1024 * 1024;

fn json_object_too_large() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "JSON object exceeds 4 MiB")
}

fn array_prefix(first: bool, pretty: bool) -> &'static [u8] {
    match (first, pretty) {
        (true, true) => b"[\n  ",
        (true, false) => b"[",
        (false, true) => b",\n  ",
        (false, false) => b",",
    }
}

fn array_suffix(empty: bool, pretty: bool) -> &'static [u8] {
    match (empty, pretty) {
        (true, _) => b"[]\n",
        (false, true) => b"\n]\n",
        (false, false) => b"]\n",
    }
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
        self.output
            .write_all(array_prefix(self.first, self.pretty))?;
        if self.pretty {
            write_pretty_json_row(&mut self.output, &self.row)?;
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
        self.output
            .write_all(array_suffix(self.first, self.pretty))?;
        self.output.flush()?;
        Ok(self.output)
    }
}

impl<W: Write> Write for JsonArraySink<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // JSONL already validates these rows. Forward complete objects
        // without copying them into an intermediate Vec first.
        if self.row.is_empty() {
            if let Some(row) = bytes.strip_suffix(b"\n") {
                if !row.contains(&b'\n') {
                    if row.len() > MAX_JSON_ROW_BYTES {
                        return Err(json_object_too_large());
                    }
                    self.output
                        .write_all(array_prefix(self.first, self.pretty))?;
                    if self.pretty {
                        write_pretty_json_row(&mut self.output, row)?;
                    } else {
                        self.output.write_all(row)?;
                    }
                    self.first = false;
                    return Ok(bytes.len());
                }
            }
        }
        for chunk in bytes.split_inclusive(|&byte| byte == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            let fragment = if complete {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            if fragment.len() > MAX_JSON_ROW_BYTES.saturating_sub(self.row.len()) {
                return Err(json_object_too_large());
            }
            if complete && self.row.is_empty() {
                self.output
                    .write_all(array_prefix(self.first, self.pretty))?;
                if self.pretty {
                    write_pretty_json_row(&mut self.output, fragment)?;
                } else {
                    self.output.write_all(fragment)?;
                }
                self.first = false;
                continue;
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
    fn batched_json_objects_skip_row_allocation() {
        let mut sink = JsonArraySink::new(Vec::new(), false);
        sink.write_all(b"{\"a\":1}\n{\"b\":2}\n").unwrap();
        assert_eq!(sink.row.capacity(), 0);
        assert_eq!(sink.finish().unwrap(), b"[{\"a\":1},{\"b\":2}]\n");
    }

    #[test]
    fn framing_handles_empty_compact_and_pretty_arrays() {
        assert_eq!(array_prefix(true, false), &b"["[..]);
        assert_eq!(array_prefix(false, false), &b","[..]);
        assert_eq!(array_prefix(true, true), &b"[\n  "[..]);
        assert_eq!(array_prefix(false, true), &b",\n  "[..]);
        assert_eq!(array_suffix(true, false), &b"[]\n"[..]);
        assert_eq!(array_suffix(true, true), &b"[]\n"[..]);
        assert_eq!(array_suffix(false, false), &b"]\n"[..]);
        assert_eq!(array_suffix(false, true), &b"\n]\n"[..]);
    }

    #[test]
    fn writes_streamed_json_array_with_escaped_values() {
        let mut writer = JsonArrayWriter::new(Vec::new());
        writer.write_all(b"number\tname\n12\t").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"\"x\"\n13\t\\N\n").unwrap();
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

    #[test]
    fn complete_json_object_avoids_intermediate_row_copy() {
        let mut sink = JsonArraySink::new(Vec::new(), false);
        sink.write_all(b"{\"key\":\"value\"}\n").unwrap();
        assert_eq!(sink.row.capacity(), 0);
        assert_eq!(sink.finish().unwrap(), b"[{\"key\":\"value\"}]\n");
    }
}
