use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Convert tab-separated numeric query output to one JSON object per row.
/// Values are strings to preserve UInt128/Int128 precision. NULL becomes JSON null.
pub struct JsonlWriter<W: Write> {
    output: W,
    headers: Option<Vec<Vec<u8>>>,
    line: Vec<u8>,
}

impl<W: Write> JsonlWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            headers: None,
            line: Vec::new(),
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        let fields: Vec<Vec<u8>> = self
            .line
            .split(|byte| *byte == b'\t')
            .map(Vec::from)
            .collect();
        self.line.clear();
        if self.headers.is_none() {
            for (index, header) in fields.iter().enumerate() {
                if fields[..index].contains(header) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "JSONL requires unique projected column names",
                    ));
                }
            }
            self.headers = Some(fields);
            return Ok(());
        }
        let headers = self.headers.as_ref().unwrap();
        if fields.len() != headers.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSONL row does not match header column count",
            ));
        }
        self.output.write_all(b"{")?;
        for (index, (header, value)) in headers.iter().zip(fields.iter()).enumerate() {
            if index > 0 {
                self.output.write_all(b",")?;
            }
            write_json_string(&mut self.output, header)?;
            self.output.write_all(b":")?;
            if value == b"NULL" {
                self.output.write_all(b"null")?;
            } else {
                write_json_string(&mut self.output, value)?;
            }
        }
        self.output.write_all(b"}\n")
    }
}

fn write_json_string<W: Write>(writer: &mut W, input: &[u8]) -> io::Result<()> {
    writer.write_all(b"\"")?;
    for ch in String::from_utf8_lossy(input).chars() {
        match ch {
            '"' => writer.write_all(b"\\\"")?,
            '\\' => writer.write_all(b"\\\\")?,
            '\n' => writer.write_all(b"\\n")?,
            '\r' => writer.write_all(b"\\r")?,
            '\t' => writer.write_all(b"\\t")?,
            ch if ch.is_control() => write!(writer, "\\u{:04x}", ch as u32)?,
            ch => {
                let mut buf = [0; 4];
                writer.write_all(ch.encode_utf8(&mut buf).as_bytes())?;
            }
        }
    }
    writer.write_all(b"\"")
}

impl<W: Write> Write for JsonlWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_line()?;
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "JSONL row exceeds 1 MiB",
                    ));
                }
                self.line.push(byte);
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
    fn writes_escaped_json_and_preserves_large_values() {
        let mut output = Vec::new();
        {
            let mut writer = JsonlWriter::new(&mut output);
            writer
                .write_all(b"name\tamount\n\"a\"\t340282366920938463463374607431768211455\n")
                .unwrap();
            writer.write_all(b"missing\tNULL\n").unwrap();
            writer.flush().unwrap();
        }
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "{\"name\":\"\\\"a\\\"\",\"amount\":\"340282366920938463463374607431768211455\"}\n{\"name\":\"missing\",\"amount\":null}\n"
        );
    }

    #[test]
    fn rejects_duplicate_headers_and_incomplete_rows() {
        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(b"a\ta\n").is_err());

        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"a\tb\n").unwrap();
        assert!(writer.write_all(b"1\n").is_err());
    }

    #[test]
    fn rejects_oversized_rows() {
        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(&vec![b'a'; MAX_LINE_BYTES + 1]).is_err());
    }

    #[test]
    fn flush_does_not_split_headers_or_rows() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"name\tamount").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"\nabc\t1").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"2\n").unwrap();
        assert_eq!(
            String::from_utf8(writer.output).unwrap(),
            "{\"name\":\"abc\",\"amount\":\"12\"}\n"
        );
    }
}
