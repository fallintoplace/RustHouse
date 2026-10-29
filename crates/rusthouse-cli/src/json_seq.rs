//! RFC 7464 JSON text sequences built on the existing escaped JSONL writer.
//! Every JSON object is preceded by an ASCII record separator (0x1E).
use crate::jsonl::JsonlWriter;
use std::io::{self, Write};

struct JsonSeqSink<W: Write> {
    output: W,
    at_row_start: bool,
}

impl<W: Write> JsonSeqSink<W> {
    fn new(output: W) -> Self {
        Self {
            output,
            at_row_start: true,
        }
    }

    fn write_record_fragment(&mut self, chunk: &[u8]) -> io::Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        if self.at_row_start {
            self.output.write_all(b"\x1e")?;
            self.at_row_start = false;
        }
        self.output.write_all(chunk)?;
        if chunk.ends_with(b"\n") {
            self.at_row_start = true;
        }
        Ok(())
    }
}

impl<W: Write> Write for JsonSeqSink<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for chunk in bytes.split_inclusive(|&byte| byte == b'\n') {
            self.write_record_fragment(chunk)?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

pub struct JsonSeqWriter<W: Write> {
    inner: JsonlWriter<JsonSeqSink<W>>,
}

impl<W: Write> JsonSeqWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            inner: JsonlWriter::new(JsonSeqSink::new(output)),
        }
    }
}

impl<W: Write> Write for JsonSeqWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_record_separators_across_fragmented_writes() {
        let mut sink = JsonSeqSink::new(Vec::new());
        sink.write_all(b"{\"key\":").unwrap();
        sink.write_all(b"\"one\"}\n{\"key\":\"two\"}\n").unwrap();
        sink.write_all(b"{\"key\":\"three\"}\n").unwrap();
        assert_eq!(
            sink.output,
            b"\x1e{\"key\":\"one\"}\n\x1e{\"key\":\"two\"}\n\x1e{\"key\":\"three\"}\n"
        );
    }

    #[test]
    fn skips_headers_and_produces_empty_sequence_for_no_rows() {
        let mut writer = JsonSeqWriter::new(Vec::new());
        writer.write_all(b"name\n").unwrap();
        assert_eq!(writer.inner.into_inner().output, b"");

        let mut writer = JsonSeqWriter::new(Vec::new());
        writer.write_all(b"name\nvalue\n").unwrap();
        assert_eq!(
            writer.inner.into_inner().output,
            b"\x1e{\"name\":\"value\"}\n"
        );
    }
}
