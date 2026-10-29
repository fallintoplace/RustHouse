//! Convert the SQL executor's TSV stream into a bounded, escaped HTML table.
use crate::tsv_escape::decode_tsv_field;
use std::borrow::Cow;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

pub struct HtmlWriter<W: Write> {
    output: W,
    line: Vec<u8>,
    columns: Option<usize>,
}

impl<W: Write> HtmlWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            line: Vec::new(),
            columns: None,
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        let text = std::str::from_utf8(&self.line).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "HTML field is not valid UTF-8")
        })?;
        let field_count = 1 + self.line.iter().filter(|&&byte| byte == b'\t').count();
        let header = self.columns.is_none();
        if let Some(columns) = self.columns {
            if columns != field_count {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTML row does not match header column count",
                ));
            }
        }

        // Headers are literal and unescaped data can be emitted directly.
        // Decode the whole row first only when it actually contains TSV escapes.
        let decoded = if header || !text.as_bytes().contains(&b'\\') {
            None
        } else {
            Some(decode_html_fields(text, header)?)
        };

        if header {
            self.output.write_all(b"<table>\n<thead><tr>")?;
        } else {
            self.output.write_all(b"<tr>")?;
        }
        if let Some(fields) = decoded {
            write_cells(&mut self.output, &fields, header)?;
        } else {
            write_plain_cells(&mut self.output, text, header)?;
        }
        if header {
            self.output.write_all(b"</tr></thead>\n<tbody>\n")?;
            self.columns = Some(field_count);
        } else {
            self.output.write_all(b"</tr>\n")?;
        }
        self.line.clear();
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<W> {
        if !self.line.is_empty() || self.columns.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTML result has an incomplete line or missing header",
            ));
        }
        self.output.write_all(b"</tbody>\n</table>\n")?;
        self.output.flush()?;
        Ok(self.output)
    }
}

fn decode_tsv_field_for_html(field: &str, header: bool) -> io::Result<Cow<'_, str>> {
    if header || field == "\\N" || !field.as_bytes().contains(&b'\\') {
        return Ok(Cow::Borrowed(field));
    }
    String::from_utf8(decode_tsv_field(field.as_bytes())?)
        .map(Cow::Owned)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "HTML field is not valid UTF-8"))
}

fn decode_html_fields(line: &str, header: bool) -> io::Result<Vec<Cow<'_, str>>> {
    line.split('\t')
        .map(|field| decode_tsv_field_for_html(field, header))
        .collect()
}

fn write_cells<W: Write>(output: &mut W, fields: &[Cow<'_, str>], header: bool) -> io::Result<()> {
    let (open, close) = if header {
        (b"<th>".as_slice(), b"</th>".as_slice())
    } else {
        (b"<td>".as_slice(), b"</td>".as_slice())
    };
    for field in fields {
        output.write_all(open)?;
        write_cell(output, field)?;
        output.write_all(close)?;
    }
    Ok(())
}

fn write_plain_cells<W: Write>(output: &mut W, line: &str, header: bool) -> io::Result<()> {
    let (open, close) = if header {
        (b"<th>".as_slice(), b"</th>".as_slice())
    } else {
        (b"<td>".as_slice(), b"</td>".as_slice())
    };
    for field in line.split('\t') {
        output.write_all(open)?;
        write_cell(output, field)?;
        output.write_all(close)?;
    }
    Ok(())
}

fn html_escape_entity(ch: char) -> Option<&'static [u8]> {
    match ch {
        '&' => Some(b"&amp;"),
        '<' => Some(b"&lt;"),
        '>' => Some(b"&gt;"),
        '"' => Some(b"&quot;"),
        '\'' => Some(b"&#39;"),
        _ => None,
    }
}

fn write_cell<W: Write>(output: &mut W, field: &str) -> io::Result<()> {
    let mut start = 0;
    for (index, ch) in field.char_indices() {
        if ch.is_control() {
            if start < index {
                output.write_all(&field.as_bytes()[start..index])?;
            }
            write!(output, "&#{};", ch as u32)?;
            start = index + ch.len_utf8();
            continue;
        }
        if let Some(escape) = html_escape_entity(ch) {
            if start < index {
                output.write_all(&field.as_bytes()[start..index])?;
            }
            output.write_all(escape)?;
            start = index + ch.len_utf8();
        }
    }
    if start < field.len() {
        output.write_all(&field.as_bytes()[start..])?;
    }
    Ok(())
}

impl<W: Write> Write for HtmlWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for chunk in bytes.split_inclusive(|&byte| byte == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            let fragment = if complete {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            if fragment.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTML row exceeds 1 MiB",
                ));
            }
            self.line.extend_from_slice(fragment);
            if complete {
                self.finish_line()?;
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
    fn plain_cells_keep_utf8_and_escape_html_without_field_collection() {
        let mut output = Vec::new();
        write_plain_cells(&mut output, "café\t<a>", false).unwrap();
        assert_eq!(output, "<td>café</td><td>&lt;a&gt;</td>".as_bytes());
    }

    #[test]
    fn processes_many_rows_from_one_bulk_write() {
        let mut writer = HtmlWriter::new(Vec::new());
        let input = format!("value\n{}", "42\n".repeat(1_000));
        writer.write_all(input.as_bytes()).unwrap();
        let html = String::from_utf8(writer.finish().unwrap()).unwrap();
        assert_eq!(html.matches("<tr>").count(), 1_001);
        assert_eq!(html.matches("<td>42</td>").count(), 1_000);
    }

    #[test]
    fn field_decoder_borrows_plain_headers_and_null_markers() {
        assert!(matches!(
            decode_tsv_field_for_html("plain", false).unwrap(),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            decode_tsv_field_for_html("\\N", false).unwrap(),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            decode_tsv_field_for_html("a\\tb", true).unwrap(),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            decode_tsv_field_for_html("a\\tb", false).unwrap(),
            Cow::Owned(_)
        ));
    }

    #[test]
    fn only_decodes_escaped_data_fields() {
        let headers = decode_html_fields(r"name\label", true).unwrap();
        assert!(matches!(&headers[0], Cow::Borrowed(_)));
        let cells = decode_html_fields("\\N\tdecoded\\x20text", false).unwrap();
        assert!(matches!(&cells[0], Cow::Borrowed(_)));
        assert_eq!(cells[1].as_ref(), "decoded text");
    }

    #[test]
    fn escapes_untrusted_cell_content_and_keeps_unicode() {
        let mut writer = HtmlWriter::new(Vec::new());
        writer.write_all(b"name\tvalue\n").unwrap();
        writer.flush().unwrap();
        writer
            .write_all("<script>\tTom & \"Jerry\" 'é'\r\u{001b}\n".as_bytes())
            .unwrap();
        let result = String::from_utf8(writer.finish().unwrap()).unwrap();
        assert_eq!(
            result,
            "<table>\n<thead><tr><th>name</th><th>value</th></tr></thead>\n<tbody>\n<tr><td>&lt;script&gt;</td><td>Tom &amp; &quot;Jerry&quot; &#39;é&#39;&#13;&#27;</td></tr>\n</tbody>\n</table>\n"
        );
    }

    #[test]
    fn finishes_empty_tables_and_rejects_incomplete_rows() {
        let mut writer = HtmlWriter::new(Vec::new());
        writer.write_all(b"number\n").unwrap();
        assert_eq!(
            String::from_utf8(writer.finish().unwrap()).unwrap(),
            "<table>\n<thead><tr><th>number</th></tr></thead>\n<tbody>\n</tbody>\n</table>\n"
        );
        let mut writer = HtmlWriter::new(Vec::new());
        writer.write_all(b"number\n10").unwrap();
        assert!(writer.finish().is_err());
    }

    #[test]
    fn rejects_invalid_utf8_and_wrong_column_counts() {
        let mut writer = HtmlWriter::new(Vec::new());
        writer.write_all(b"first\tsecond\n").unwrap();
        assert!(writer.write_all(b"one\n").is_err());

        let mut writer = HtmlWriter::new(Vec::new());
        writer.write_all(b"field\n").unwrap();
        assert!(writer.write_all(b"\xff\n").is_err());
    }

    #[test]
    fn renders_header_and_data_cell_tags() {
        let mut output = Vec::new();
        write_cells(&mut output, &["name".into(), "value".into()], true).unwrap();
        write_cells(&mut output, &["id".into(), "1".into()], false).unwrap();
        assert_eq!(output, b"<th>name</th><th>value</th><td>id</td><td>1</td>");
    }

    #[test]
    fn caps_incomplete_lines() {
        let mut writer = HtmlWriter::new(Vec::new());
        assert!(writer.write_all(&vec![b'x'; MAX_LINE_BYTES + 1]).is_err());
    }
}

#[cfg(test)]
mod decoded_html_regressions {
    use super::*;

    #[test]
    fn decodes_sql_string_escapes_before_html_escaping() {
        let mut writer = HtmlWriter::new(Vec::new());
        writer
            .write_all(b"name\nline\\nnext\\tcell\\\\tail\n")
            .unwrap();
        let html = String::from_utf8(writer.finish().unwrap()).unwrap();
        assert!(html.contains(r"<td>line&#10;next&#9;cell\tail</td>"));
    }

    #[test]
    fn rejects_bad_tsv_escape_before_starting_the_row() {
        let mut writer = HtmlWriter::new(Vec::new());
        writer.write_all(b"name\n").unwrap();
        let before = writer.output.clone();
        assert!(writer.write_all(b"bad\\q\n").is_err());
        assert_eq!(writer.output, before);
    }
}

#[cfg(test)]
mod html_bulk_perf_tests {
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
    fn writes_plain_utf8_text_in_one_call() {
        let long = "café".repeat(4096);
        let mut output = CountingWriter { calls: 0, bytes: 0 };
        write_cell(&mut output, &long).unwrap();
        assert_eq!(output.bytes, long.len());
        assert_eq!(output.calls, 1);
    }

    #[test]
    fn bulk_escaping_keeps_unicode_and_controls() {
        let mut output = Vec::new();
        write_cell(&mut output, "ab<&>é\u{0001}z").unwrap();
        assert_eq!(output, "ab&lt;&amp;&gt;é&#1;z".as_bytes());
    }
}
