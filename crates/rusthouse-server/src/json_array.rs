//! Stream validated JSONL objects as one bounded JSON array.
use super::MAX_RESPONSE_BYTES;
use rusthouse_format::write_pretty_json_row;
use std::io::{self, Write};

fn validate_complete_object(row: &[u8]) -> io::Result<()> {
    if !row.starts_with(b"{") || !row.ends_with(b"}") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a complete JSON object",
        ));
    }
    Ok(())
}

pub(super) struct JsonArraySink<W: Write> {
    output: W,
    row: Vec<u8>,
    first: bool,
    pretty: bool,
}

impl<W: Write> JsonArraySink<W> {
    pub(super) fn new(output: W) -> Self {
        Self::with_style(output, false)
    }

    pub(super) fn new_pretty(output: W) -> Self {
        Self::with_style(output, true)
    }

    fn with_style(output: W, pretty: bool) -> Self {
        Self {
            output,
            row: Vec::new(),
            first: true,
            pretty,
        }
    }

    fn write_row_prefix(&mut self) -> io::Result<()> {
        let prefix: &[u8] = match (self.first, self.pretty) {
            (true, true) => b"[\n  ",
            (true, false) => b"[",
            (false, true) => b",\n  ",
            (false, false) => b",",
        };
        self.output.write_all(prefix)?;
        self.first = false;
        Ok(())
    }

    fn append_row_fragment(&mut self, fragment: &[u8]) -> io::Result<()> {
        if fragment.len() > MAX_RESPONSE_BYTES.saturating_sub(self.row.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSON array row exceeds 8 MiB",
            ));
        }
        self.row.extend_from_slice(fragment);
        Ok(())
    }

    fn finish_row(&mut self) -> io::Result<()> {
        validate_complete_object(&self.row)?;
        self.write_row_prefix()?;
        if self.pretty {
            write_pretty_json_row(&mut self.output, &self.row)?;
        } else {
            self.output.write_all(&self.row)?;
        }
        self.row.clear();
        Ok(())
    }

    pub(super) fn finish(mut self) -> io::Result<W> {
        if !self.row.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete JSON array row",
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
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // The common complete single-row write can be forwarded directly.
        // Fragmented and multi-row writes still use the bounded row buffer.
        if self.row.is_empty() {
            if let Some(row) = buf.strip_suffix(b"\n") {
                if !row.contains(&b'\n') {
                    if row.len() > MAX_RESPONSE_BYTES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "JSON array row exceeds 8 MiB",
                        ));
                    }
                    validate_complete_object(row)?;
                    self.write_row_prefix()?;
                    if self.pretty {
                        write_pretty_json_row(&mut self.output, row)?;
                    } else {
                        self.output.write_all(row)?;
                    }
                    return Ok(buf.len());
                }
            }
        }
        for chunk in buf.split_inclusive(|&byte| byte == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            let fragment = if complete {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            if complete && self.row.is_empty() {
                if fragment.len() > MAX_RESPONSE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "JSON array row exceeds 8 MiB",
                    ));
                }
                validate_complete_object(fragment)?;
                self.write_row_prefix()?;
                if self.pretty {
                    write_pretty_json_row(&mut self.output, fragment)?;
                } else {
                    self.output.write_all(fragment)?;
                }
                continue;
            }
            self.append_row_fragment(fragment)?;
            if complete {
                self.finish_row()?;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_multirow_writes_do_not_allocate_a_row_buffer() {
        let mut sink = JsonArraySink::new(Vec::new());
        sink.write_all(b"{\"a\":1}\n{\"b\":2}\n").unwrap();
        assert_eq!(sink.row.capacity(), 0);
        assert_eq!(sink.finish().unwrap(), b"[{\"a\":1},{\"b\":2}]\n");
    }

    #[test]
    fn pretty_json_streams_fragmented_objects_and_handles_escaped_punctuation() {
        let mut sink = JsonArraySink::new_pretty(Vec::new());
        sink.write_all(br#"{"text":"a,{}\\\"b"}"#).unwrap();
        sink.write_all(b"\n").unwrap();
        sink.write_all(b"{\"x\":null}\n").unwrap();
        let result = String::from_utf8(sink.finish().unwrap()).unwrap();
        assert!(result.starts_with("[\n  {\n    \"text\": "));
        assert!(result.contains(r#"a,{}\\\"b"#));
        assert!(result.contains(",\n  {\n    \"x\": null\n  }\n]\n"));
        assert_eq!(
            JsonArraySink::new_pretty(Vec::new()).finish().unwrap(),
            b"[]\n"
        );
    }

    #[test]
    fn supports_fragmented_json_objects_without_exposing_partial_rows() {
        let mut sink = JsonArraySink::new(Vec::new());
        sink.write_all(b"{\"key\":\"a\"}").unwrap();
        sink.flush().unwrap();
        sink.write_all(b"\n{\"key\":null}\n").unwrap();
        assert_eq!(
            sink.finish().unwrap(),
            b"[{\"key\":\"a\"},{\"key\":null}]\n"
        );
    }

    #[test]
    fn closes_empty_results() {
        assert_eq!(JsonArraySink::new(Vec::new()).finish().unwrap(), b"[]\n");
    }

    #[test]
    fn rejects_truncated_or_non_object_rows() {
        let mut sink = JsonArraySink::new(Vec::new());
        sink.write_all(b"{}").unwrap();
        assert!(sink.finish().is_err());

        let mut sink = JsonArraySink::new(Vec::new());
        assert!(sink.write_all(b"not-object\n").is_err());
    }

    #[test]
    fn enforces_response_bound_on_individual_rows() {
        let mut sink = JsonArraySink::new(Vec::new());
        assert!(sink.write_all(&vec![b'a'; MAX_RESPONSE_BYTES + 1]).is_err());
    }
    #[test]
    fn buffers_large_rows_and_multiple_row_boundaries_in_one_write() {
        let large_row = format!("{{\"value\":\"{}\"}}", "x".repeat(64 * 1024));
        let mut sink = JsonArraySink::new(Vec::new());
        sink.write_all(large_row.as_bytes()).unwrap();
        sink.write_all(b"\n{}\n").unwrap();
        assert_eq!(
            sink.finish().unwrap(),
            format!("[{large_row},{{}}]\n").as_bytes()
        );
    }

    #[test]
    fn complete_json_row_skips_temporary_buffering() {
        let input = format!("{{\"label\":\"{}\"}}\n", "x".repeat(4096));
        let mut sink = JsonArraySink::new(Vec::new());
        sink.write_all(input.as_bytes()).unwrap();
        assert_eq!(sink.row.capacity(), 0);
        let output = sink.finish().unwrap();
        assert_eq!(output, format!("[{}]\n", input.trim_end()).as_bytes());
    }
}
