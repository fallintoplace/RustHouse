use rusthouse_format::decode_tsv_field;
use std::borrow::Cow;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn finish_complete_line<W: Write>(
    output: &mut W,
    columns: &mut Option<usize>,
    line: &[u8],
) -> io::Result<()> {
    let width = 1 + line.iter().filter(|&&byte| byte == b'\t').count();
    if columns.is_some_and(|expected| expected != width) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Markdown row does not match header column count",
        ));
    }
    write_markdown_row(output, line, columns.is_some())?;
    if columns.is_none() {
        write_header_separator(output, width)?;
        *columns = Some(width);
    }
    Ok(())
}

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
        finish_complete_line(&mut self.output, &mut self.columns, &self.line)?;
        self.line.clear();
        Ok(())
    }
}

fn decode_tsv_markdown_field(field: &[u8], data_row: bool) -> io::Result<Cow<'_, [u8]>> {
    if data_row && field != b"\\N" && field.contains(&b'\\') {
        let decoded = decode_tsv_field(field)?;
        std::str::from_utf8(&decoded).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "Markdown field is not UTF-8")
        })?;
        Ok(Cow::Owned(decoded))
    } else {
        Ok(Cow::Borrowed(field))
    }
}

fn write_markdown_row<W: Write>(output: &mut W, line: &[u8], data_row: bool) -> io::Result<()> {
    if !line.contains(&b'\\') {
        // Validate before writing, so malformed UTF-8 never leaves a partial row.
        std::str::from_utf8(line).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "Markdown field is not UTF-8")
        })?;
        output.write_all(b"|")?;
        for field in line.split(|&byte| byte == b'\t') {
            output.write_all(b" ")?;
            write_cell(output, field)?;
            output.write_all(b" |")?;
        }
        return output.write_all(b"\n");
    }
    let fields = line
        .split(|&byte| byte == b'\t')
        .map(|field| decode_tsv_markdown_field(field, data_row))
        .collect::<io::Result<Vec<_>>>()?;
    for field in &fields {
        std::str::from_utf8(field.as_ref()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "Markdown field is not UTF-8")
        })?;
    }
    output.write_all(b"|")?;
    for field in fields {
        output.write_all(b" ")?;
        write_cell(output, field.as_ref())?;
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
    let mut start = 0;
    for (index, &byte) in field.iter().enumerate() {
        let escape: Option<&[u8]> = match byte {
            b'|' => Some(b"\\|"),
            b'\\' => Some(b"\\\\"),
            b'&' => Some(b"&amp;"),
            b'<' => Some(b"&lt;"),
            b'>' => Some(b"&gt;"),
            b'\r' => Some(b"&#13;"),
            byte if byte.is_ascii_control() => {
                if start < index {
                    output.write_all(&field[start..index])?;
                }
                write!(output, "&#{byte};")?;
                start = index + 1;
                continue;
            }
            _ => None,
        };
        if let Some(escape) = escape {
            if start < index {
                output.write_all(&field[start..index])?;
            }
            output.write_all(escape)?;
            start = index + 1;
        }
    }
    if start < field.len() {
        output.write_all(&field[start..])?;
    }
    Ok(())
}

impl<W: Write> Write for MarkdownWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for chunk in buf.split_inclusive(|&byte| byte == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            let fragment = if complete {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            if fragment.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Markdown row exceeds 1 MiB",
                ));
            }
            if complete && self.line.is_empty() {
                finish_complete_line(&mut self.output, &mut self.columns, fragment)?;
                continue;
            }
            self.line.extend_from_slice(fragment);
            if complete {
                self.finish_line()?;
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
    fn complete_rows_bypass_the_line_staging_buffer() {
        let mut writer = MarkdownWriter::new(Vec::new());
        writer.write_all(b"item\n").unwrap();
        let before = writer.line.capacity();
        let item = "z".repeat(16 * 1024);
        writer.write_all(format!("{item}\n").as_bytes()).unwrap();
        assert_eq!(writer.line.capacity(), before);
        assert!(writer.output.ends_with(format!("| {item} |\n").as_bytes()));
    }

    #[test]
    fn plain_markdown_rows_stream_without_escape_decoding() {
        let mut output = Vec::new();
        write_markdown_row(&mut output, "café\t42".as_bytes(), true).unwrap();
        assert_eq!(output, "| café | 42 |\n".as_bytes());
    }

    #[test]
    fn field_decoder_borrows_plain_and_null_fields() {
        assert!(matches!(
            decode_tsv_markdown_field(b"plain", true).unwrap(),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            decode_tsv_markdown_field(b"\\N", true).unwrap(),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            decode_tsv_markdown_field(b"a\\tb", false).unwrap(),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            decode_tsv_markdown_field(b"a\\tb", true).unwrap(),
            Cow::Owned(_)
        ));
    }

    #[test]
    fn shared_decoder_validates_unicode_and_bad_bytes() {
        assert_eq!(
            decode_tsv_markdown_field(br"caf\xC3\xA9", true)
                .unwrap()
                .as_ref(),
            "café".as_bytes()
        );
        assert!(decode_tsv_markdown_field(br"\xFF", true).is_err());
    }

    #[test]
    fn row_renderer_keeps_cell_escaping_independent_of_writer_state() {
        let mut output = Vec::new();
        write_markdown_row(&mut output, b"first|col\t<a>&b", false).unwrap();
        assert_eq!(output, b"| first\\|col | &lt;a&gt;&amp;b |\n");
    }

    #[test]
    fn handles_chunked_writes_and_flush_without_splitting_rows() {
        let mut writer = MarkdownWriter::new(Vec::new());
        writer.write_all(b"a|b\tpath\nx").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"y\tz\\\\q\n").unwrap();
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
    fn decodes_tsv_escapes_without_corrupting_markdown_cells() {
        let mut writer = MarkdownWriter::new(Vec::new());
        writer
            .write_all(b"value\none\\ttwo\\nthree\\\\tail\n")
            .unwrap();
        assert_eq!(
            writer.output,
            b"| value |\n| --- |\n| one&#9;two&#10;three\\\\tail |\n"
        );

        let mut writer = MarkdownWriter::new(Vec::new());
        writer.write_all(b"col\n").unwrap();
        let before = writer.output.clone();
        assert!(writer.write_all(b"bad\\q\n").is_err());
        assert_eq!(writer.output, before);
    }

    #[test]
    fn rejects_invalid_utf8_without_writing_partial_rows() {
        let mut writer = MarkdownWriter::new(Vec::new());
        assert_eq!(
            writer.write_all(b"\xff\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(writer.output.is_empty());

        let mut writer = MarkdownWriter::new(Vec::new());
        writer.write_all(b"label\n").unwrap();
        let previous = writer.output.clone();
        assert_eq!(
            writer.write_all(b"\xff\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(writer.output, previous);
    }

    #[test]
    fn accepts_exact_row_limit_in_fragmented_writes() {
        let row = vec![b'a'; MAX_LINE_BYTES];
        let mut writer = MarkdownWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        writer.write_all(&row[..MAX_LINE_BYTES / 2]).unwrap();
        writer.write_all(&row[MAX_LINE_BYTES / 2..]).unwrap();
        writer.write_all(b"\n").unwrap();
        assert!(writer.output.ends_with(b" |\n"));

        let mut writer = MarkdownWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        writer.write_all(&row).unwrap();
        assert_eq!(
            writer.write_all(b"a").unwrap_err().kind(),
            io::ErrorKind::InvalidData
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

#[cfg(test)]
mod markdown_bulk_output_tests {
    use super::*;
    struct CountingWriter {
        calls: usize,
        bytes: usize,
    }
    impl Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn writes_plain_markdown_cell_with_one_bulk_call() {
        let mut output = CountingWriter { calls: 0, bytes: 0 };
        write_cell(&mut output, &vec![b'a'; 16 * 1024]).unwrap();
        assert_eq!(output.bytes, 16 * 1024);
        assert_eq!(output.calls, 1);
    }
    #[test]
    fn keeps_escaped_bytes_and_control_formatting() {
        let mut output = Vec::new();
        write_cell(&mut output, b"abc|xyz<&>\r\x01").unwrap();
        assert_eq!(output, b"abc\\|xyz&lt;&amp;&gt;&#13;&#1;");
    }
}
