use crate::jsonl::JsonlWriter;
use std::io::{self, Write};

const MAX_JSON_ROW_BYTES: usize = 4 * 1024 * 1024;

/// Convert a stream of JSONL objects to a valid JSON array without buffering all rows.
struct JsonArraySink<W: Write> {
    output: W,
    row: Vec<u8>,
    started: bool,
    first: bool,
}

impl<W: Write> JsonArraySink<W> {
    fn new(output: W) -> Self {
        Self {
            output,
            row: Vec::new(),
            started: false,
            first: true,
        }
    }

    fn finish_row(&mut self) -> io::Result<()> {
        if !self.started {
            self.output.write_all(b"[")?;
            self.started = true;
        }
        if !self.first {
            self.output.write_all(b",")?;
        }
        self.output.write_all(&self.row)?;
        self.row.clear();
        self.first = false;
        Ok(())
    }
}

impl<W: Write> Write for JsonArraySink<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_row()?;
            } else {
                if self.row.len() >= MAX_JSON_ROW_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "JSON object exceeds 4 MiB",
                    ));
                }
                self.row.push(byte);
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
        Self {
            inner: JsonlWriter::new(JsonArraySink::new(output)),
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
        let mut sink = self.inner.into_inner();
        if !sink.row.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete JSON row",
            ));
        }
        if sink.started {
            sink.output.write_all(b"]\n")?;
        } else {
            sink.output.write_all(b"[]\n")?;
        }
        sink.output.flush()?;
        Ok(sink.output)
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
}
