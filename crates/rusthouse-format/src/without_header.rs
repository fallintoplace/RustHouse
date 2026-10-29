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
        let payload = if self.skipping_header {
            let Some(index) = buf.iter().position(|byte| *byte == b'\n') else {
                return Ok(buf.len());
            };
            self.skipping_header = false;
            &buf[index + 1..]
        } else {
            buf
        };
        self.output.write_all(payload)?;
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
