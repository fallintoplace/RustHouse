//! Stream validated JSONL objects as one bounded JSON array.
use super::MAX_RESPONSE_BYTES;
use std::io::{self, Write};

pub(super) struct JsonArraySink<W: Write> {
    output: W,
    row: Vec<u8>,
    first: bool,
}

impl<W: Write> JsonArraySink<W> {
    pub(super) fn new(output: W) -> Self {
        Self {
            output,
            row: Vec::new(),
            first: true,
        }
    }

    fn finish_row(&mut self) -> io::Result<()> {
        if !self.row.starts_with(b"{") || !self.row.ends_with(b"}") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected a complete JSON object",
            ));
        }
        if self.first {
            self.output.write_all(b"[")?;
            self.first = false;
        } else {
            self.output.write_all(b",")?;
        }
        self.output.write_all(&self.row)?;
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
        } else {
            self.output.write_all(b"]\n")?;
        }
        self.output.flush()?;
        Ok(self.output)
    }
}

impl<W: Write> Write for JsonArraySink<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for chunk in buf.split_inclusive(|&byte| byte == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            let fragment = if complete {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            if fragment.len() > MAX_RESPONSE_BYTES.saturating_sub(self.row.len()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "JSON array row exceeds 8 MiB",
                ));
            }
            self.row.extend_from_slice(fragment);
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

}
