use std::io::{self, Write};

/// Discard exactly the first completed CSV record (the SELECT column names).
/// Handles writes split at any byte boundary, including CRLF boundaries.
pub(super) struct WithoutHeader<W: Write> {
    output: W,
    skipping_header: bool,
}

impl<W: Write> WithoutHeader<W> {
    pub(super) fn new(output: W) -> Self {
        Self {
            output,
            skipping_header: true,
        }
    }
}

impl<W: Write> Write for WithoutHeader<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.skipping_header {
            if let Some(index) = buf.iter().position(|byte| *byte == b'\n') {
                self.skipping_header = false;
                self.output.write_all(&buf[index + 1..])?;
            }
        } else {
            self.output.write_all(buf)?;
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
    fn strips_header_across_split_crlf_and_keeps_quoted_rows() {
        let mut sink = WithoutHeader::new(Vec::new());
        sink.write_all(b"id,value\r").unwrap();
        sink.flush().unwrap();
        sink.write_all(b"\n1,\"one,two\"\r\n2,three\r\n").unwrap();
        assert_eq!(sink.output, b"1,\"one,two\"\r\n2,three\r\n");
    }

    #[test]
    fn does_not_emit_headers_for_empty_results() {
        let mut sink = WithoutHeader::new(Vec::new());
        sink.write_all(b"id,value\r\n").unwrap();
        assert!(sink.output.is_empty());
    }
}
