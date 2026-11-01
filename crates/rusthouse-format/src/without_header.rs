use std::io::{self, Write};

/// Drop the first newline-terminated header row from TSV or converted CSV output.
pub struct WithoutHeader<W: Write> {
    output: W,
    skipping_header: bool,
}

impl<W: Write> WithoutHeader<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            skipping_header: true,
        }
    }
}

impl<W: Write> Write for WithoutHeader<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.skipping_header {
            return self.output.write(buf);
        }
        let Some(header_end) = buf.iter().position(|&byte| byte == b'\n') else {
            return Ok(buf.len());
        };
        let body = &buf[header_end + 1..];
        if body.is_empty() {
            self.skipping_header = false;
            return Ok(buf.len());
        }
        // Report exactly what the underlying writer accepted. A short write
        // must not discard the remainder of the first data row.
        let written = self.output.write(body)?;
        // Commit the header transition only after a successful write.
        // Interrupted/error writes may be retried with the original buffer.
        self.skipping_header = false;
        Ok(header_end + 1 + written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_header_across_write_boundaries() {
        let mut out = WithoutHeader::new(Vec::new());
        out.write_all(b"a,b\r").unwrap();
        out.flush().unwrap();
        out.write_all(b"\n1,2\r\n3,4\r\n").unwrap();
        assert_eq!(out.output, b"1,2\r\n3,4\r\n");
    }

    #[test]
    fn empty_results_emit_no_header() {
        let mut out = WithoutHeader::new(Vec::new());
        out.write_all(b"number\r\n").unwrap();
        assert!(out.output.is_empty());
    }

    #[test]
    fn strips_split_crlf_header_and_preserves_quoted_rows() {
        let mut writer = WithoutHeader::new(Vec::new());
        writer.write_all(b"id,value\r").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"\n1,\"one,two\"\r\n2,three\r\n").unwrap();
        assert_eq!(writer.output, b"1,\"one,two\"\r\n2,three\r\n");
    }
}

#[cfg(test)]
mod short_write_header_filter_tests {
    use super::*;

    #[derive(Default)]
    struct ShortSink { bytes: Vec<u8>, attempts: usize }
    impl Write for ShortSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.attempts += 1;
            let len = bytes.len().min(2);
            self.bytes.extend_from_slice(&bytes[..len]);
            Ok(len)
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[test]
    fn reports_exact_progress_through_short_underlying_writes() {
        let mut filtered = WithoutHeader::new(ShortSink::default());
        filtered.write_all(b"name,value\n1,abcdef\n2,xyz\n").unwrap();
        assert_eq!(filtered.output.bytes, b"1,abcdef\n2,xyz\n");
        assert!(filtered.output.attempts > 2);
    }

    #[test]
    fn interrupted_first_body_write_retries_the_full_header_safely() {
        struct InterruptOnce { first: bool, bytes: Vec<u8> }
        impl Write for InterruptOnce {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.first {
                    self.first = false;
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        let mut filtered = WithoutHeader::new(InterruptOnce { first: true, bytes: Vec::new() });
        filtered.write_all(b"header\nfirst\n").unwrap();
        assert_eq!(filtered.output.bytes, b"first\n");
    }
}
