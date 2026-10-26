use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Format tab-separated SQL output one row at a time for terminal inspection.
///
/// The first input line contains column headers and is not printed until a data
/// row arrives. Header-only results produce no output.
pub struct VerticalWriter<W: Write> {
    output: W,
    headers: Option<Vec<Vec<u8>>>,
    line: Vec<u8>,
    rows: usize,
}

fn write_visible<W: Write>(output: &mut W, bytes: &[u8]) -> io::Result<()> {
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte.is_ascii_control() {
            output.write_all(&bytes[start..index])?;
            write!(output, "\\x{byte:02X}")?;
            start = index + 1;
        }
    }
    output.write_all(&bytes[start..])
}

impl<W: Write> VerticalWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            headers: None,
            line: Vec::new(),
            rows: 0,
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        if self.headers.is_none() {
            self.headers = Some(
                self.line
                    .split(|&byte| byte == b'\t')
                    .map(|header| header.to_vec())
                    .collect(),
            );
            self.line.clear();
            return Ok(());
        }

        let headers = self.headers.as_ref().unwrap();
        let field_count = 1 + self.line.iter().filter(|&&byte| byte == b'\t').count();
        if field_count != headers.len() {
            self.line.clear();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "vertical row does not match header column count",
            ));
        }

        let row = self
            .rows
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "too many vertical rows"))?;
        writeln!(self.output, "Row {row}:")?;
        for (header, field) in headers.iter().zip(self.line.split(|&byte| byte == b'\t')) {
            self.output.write_all(b"  ")?;
            write_visible(&mut self.output, header)?;
            self.output.write_all(b": ")?;
            write_visible(&mut self.output, field)?;
            self.output.write_all(b"\n")?;
        }
        self.output.write_all(b"\n")?;
        self.rows = row;
        self.line.clear();
        Ok(())
    }
}

impl<W: Write> Write for VerticalWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_line()?;
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "vertical row exceeds 1 MiB",
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
    fn prints_rows_in_column_value_form_even_with_fragmented_writes() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all(b"number\tvalue\n1\t").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"alpha\n2\tbeta\n").unwrap();
        assert_eq!(
            writer.output,
            b"Row 1:\n  number: 1\n  value: alpha\n\nRow 2:\n  number: 2\n  value: beta\n\n"
        );
    }

    #[test]
    fn escapes_terminal_controls_in_headers_and_values() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all(b"na\x1bme\nva\x00lue\x7f\n").unwrap();
        assert_eq!(
            writer.output,
            b"Row 1:\n  na\\x1Bme: va\\x00lue\\x7F\n\n"
        );
    }

    #[test]
    fn header_only_results_print_nothing() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all(b"number\n").unwrap();
        writer.flush().unwrap();
        assert!(writer.output.is_empty());
    }

    #[test]
    fn rejects_mismatched_column_counts_before_output() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all(b"a\tb\n").unwrap();
        assert_eq!(
            writer.write_all(b"one\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(writer.output.is_empty());
    }

    #[test]
    fn rejects_rows_larger_than_one_megabyte() {
        let mut writer = VerticalWriter::new(Vec::new());
        assert_eq!(
            writer
                .write_all(&vec![b'a'; MAX_LINE_BYTES + 1])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
