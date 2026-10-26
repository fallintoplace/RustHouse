use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Turn the SQL executor's TSV rows into a streaming Markdown table.
pub struct MarkdownWriter<W: Write> {
    output: W,
    line: Vec<u8>,
    columns: Option<usize>,
}

impl<W: Write> MarkdownWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            line: Vec::new(),
            columns: None,
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        let width = 1 + self.line.iter().filter(|&&byte| byte == b'\t').count();
        if self.columns.is_some_and(|expected| expected != width) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Markdown row does not match header column count",
            ));
        }
        write_markdown_row(&mut self.output, &self.line)?;
        if self.columns.is_none() {
            write_header_separator(&mut self.output, width)?;
            self.columns = Some(width);
        }
        self.line.clear();
        Ok(())
    }
}

fn write_markdown_row<W: Write>(output: &mut W, line: &[u8]) -> io::Result<()> {
    output.write_all(b"|")?;
    for field in line.split(|&byte| byte == b'\t') {
        output.write_all(b" ")?;
        write_cell(output, field)?;
        output.write_all(b" |")?;
    }
    output.write_all(b"\n")
}

fn write_header_separator<W: Write>(output: &mut W, columns: usize) -> io::Result<()> {
    output.write_all(b"|")?;
    for _ in 0..columns {
        output.write_all(b" --- |")?;
    }
    output.write_all(b"\n")
}

fn write_cell<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    for &byte in field {
        match byte {
            b'|' => output.write_all(b"\\|")?,
            b'\\' => output.write_all(b"\\\\")?,
            b'&' => output.write_all(b"&amp;")?,
            b'<' => output.write_all(b"&lt;")?,
            b'>' => output.write_all(b"&gt;")?,
            b'\r' => output.write_all(b"&#13;")?,
            byte if byte.is_ascii_control() => write!(output, "&#{byte};")?,
            other => output.write_all(&[other])?,
        }
    }
    Ok(())
}

impl<W: Write> Write for MarkdownWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for &byte in buf {
            if byte == b'\n' {
                self.finish_line()?;
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Markdown row exceeds 1 MiB",
                    ));
                }
                self.line.push(byte);
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
    fn row_renderer_keeps_cell_escaping_independent_of_writer_state() {
        let mut output = Vec::new();
        write_markdown_row(&mut output, b"first|col\t<a>&b").unwrap();
        assert_eq!(output, b"| first\\|col | &lt;a&gt;&amp;b |\n");
    }

    #[test]
    fn handles_chunked_writes_and_flush_without_splitting_rows() {
        let mut writer = MarkdownWriter::new(Vec::new());
        writer.write_all(b"a|b\tpath\nx").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"y\tz\\q\n").unwrap();
        assert_eq!(
            writer.output,
            b"| a\\|b | path |\n| --- | --- |\n| xy | z\\\\q |\n"
        );
    }

    #[test]
    fn escapes_html_and_ampersands() {
        let mut writer = MarkdownWriter::new(Vec::new());
        writer.write_all(b"<tag>\t&value\r\n").unwrap();
        assert_eq!(
            writer.output,
            b"| &lt;tag&gt; | &amp;value&#13; |\n| --- | --- |\n"
        );
    }

    #[test]
    fn escapes_terminal_control_bytes_in_cells() {
        let mut writer = MarkdownWriter::new(Vec::new());
        writer
            .write_all(b"name\tvalue\nESC\x1b\tNUL\x00DEL\x7f\n")
            .unwrap();
        assert_eq!(
            writer.output,
            b"| name | value |\n| --- | --- |\n| ESC&#27; | NUL&#0;DEL&#127; |\n"
        );
    }

    #[test]
    fn rejects_oversized_rows() {
        let mut writer = MarkdownWriter::new(Vec::new());
        assert!(writer.write_all(&vec![b'a'; MAX_LINE_BYTES + 1]).is_err());
    }
}

#[cfg(test)]
mod markdown_width_regressions {
    use super::*;

    #[test]
    fn rejects_ragged_rows_before_writing_a_partial_row() {
        let mut writer = MarkdownWriter::new(Vec::new());
        writer.write_all(b"a\tb\n").unwrap();
        let good = writer.output.clone();
        assert_eq!(
            writer.write_all(b"one\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(writer.output, good);

        let mut writer = MarkdownWriter::new(Vec::new());
        writer.write_all(b"a\tb\n").unwrap();
        assert_eq!(
            writer.write_all(b"one\ttwo\tthree\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
