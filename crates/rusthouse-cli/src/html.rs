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
        render_complete_row(&mut self.output, &mut self.columns, &self.line)?;
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

fn render_complete_row<W: Write>(
    output: &mut W,
    columns: &mut Option<usize>,
    line: &[u8],
) -> io::Result<()> {
    let text = std::str::from_utf8(line)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "HTML field is not valid UTF-8"))?;
    let field_count = 1 + line.iter().filter(|&&byte| byte == b'\t').count();
    let header = columns.is_none();
    if let Some(expected) = *columns {
        if expected != field_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTML row does not match header column count",
            ));
        }
    }
    let decoded = if header || !text.as_bytes().contains(&b'\\') {
        None
    } else {
        Some(decode_html_fields(text, header)?)
    };
    if header {
        output.write_all(b"<table>\n<thead><tr>")?;
    } else {
        output.write_all(b"<tr>")?;
    }
    if let Some(fields) = decoded {
        write_cells(output, &fields, header)?;
    } else {
        write_plain_cells(output, text, header)?;
    }
    if header {
        output.write_all(b"</tr></thead>\n<tbody>\n")?;
        *columns = Some(field_count);
    } else {
        output.write_all(b"</tr>\n")?;
    }
    Ok(())
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

fn cell_tags(header: bool) -> (&'static [u8], &'static [u8]) {
    if header {
        (b"<th>", b"</th>")
    } else {
        (b"<td>", b"</td>")
    }
}

fn write_cells<W: Write>(output: &mut W, fields: &[Cow<'_, str>], header: bool) -> io::Result<()> {
    let (open, close) = cell_tags(header);
    for field in fields {
        output.write_all(open)?;
        write_cell(output, field)?;
        output.write_all(close)?;
    }
    Ok(())
}

fn write_framed_plain_cell<const N: usize, W: Write>(
    output: &mut W,
    open: &[u8],
    field: &str,
    close: &[u8],
) -> io::Result<()> {
    let mut framed = [0u8; N];
    framed[..open.len()].copy_from_slice(open);
    let body_end = open.len() + field.len();
    framed[open.len()..body_end].copy_from_slice(field.as_bytes());
    framed[body_end..body_end + close.len()].copy_from_slice(close);
    output.write_all(&framed[..body_end + close.len()])
}

fn write_plain_cells<W: Write>(output: &mut W, line: &str, header: bool) -> io::Result<()> {
    let (open, close) = cell_tags(header);
    for field in line.split('\t') {
        // For very large cells, the escaping writer makes one scanning pass.
        // Small and medium safe UTF-8 cells get one framed output write.
        let unescaped = if field.len() > 4096 {
            false
        } else if field.is_ascii() {
            field.bytes().all(|byte| {
                !byte.is_ascii_control() && !matches!(byte, b'&' | b'<' | b'>' | b'"' | b'\'')
            })
        } else {
            field
                .chars()
                .all(|ch| !ch.is_control() && !matches!(ch, '&' | '<' | '>' | '"' | '\''))
        };
        if unescaped && field.len() <= 256 {
            write_framed_plain_cell::<265, _>(output, open, field, close)?;
        } else if unescaped {
            write_framed_plain_cell::<4105, _>(output, open, field, close)?;
        } else {
            output.write_all(open)?;
            write_cell(output, field)?;
            output.write_all(close)?;
        }
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

fn append_html_escape_span<W: Write>(
    output: &mut W,
    buffer: &mut [u8; 4096],
    used: &mut usize,
    mut bytes: &[u8],
) -> io::Result<()> {
    while !bytes.is_empty() {
        if *used == buffer.len() {
            output.write_all(buffer)?;
            *used = 0;
        }
        if *used == 0 && bytes.len() >= buffer.len() {
            // Large ordinary runs can pass through without a scratch copy.
            output.write_all(bytes)?;
            return Ok(());
        }
        let count = bytes.len().min(buffer.len() - *used);
        buffer[*used..*used + count].copy_from_slice(&bytes[..count]);
        *used += count;
        bytes = &bytes[count..];
    }
    Ok(())
}

fn write_cell<W: Write>(output: &mut W, field: &str) -> io::Result<()> {
    let mut start = 0;
    let mut buffer = [0u8; 4096];
    let mut used = 0;
    for (index, ch) in field.char_indices() {
        let mut numeric = [0u8; 16];
        let entity: Option<&[u8]> = if ch.is_control() {
            // Render numeric HTML entities without allocating a String.
            let length = {
                let mut cursor = std::io::Cursor::new(numeric.as_mut_slice());
                write!(&mut cursor, "&#{};", ch as u32)?;
                cursor.position() as usize
            };
            Some(&numeric[..length])
        } else {
            html_escape_entity(ch)
        };
        if let Some(entity) = entity {
            append_html_escape_span(
                output,
                &mut buffer,
                &mut used,
                &field.as_bytes()[start..index],
            )?;
            append_html_escape_span(output, &mut buffer, &mut used, entity)?;
            start = index + ch.len_utf8();
        }
    }
    if start == 0 {
        // Keep one direct write for a safe cell, including long UTF-8 values.
        return output.write_all(field.as_bytes());
    }
    append_html_escape_span(output, &mut buffer, &mut used, &field.as_bytes()[start..])?;
    if used != 0 {
        output.write_all(&buffer[..used])?;
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
            if complete && self.line.is_empty() {
                render_complete_row(&mut self.output, &mut self.columns, fragment)?;
                continue;
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
    fn full_html_rows_bypass_input_staging() {
        let mut writer = HtmlWriter::new(Vec::new());
        writer.write_all(b"name\n").unwrap();
        assert_eq!(writer.line.capacity(), 0);
        let text = "café".repeat(4096);
        writer.write_all(format!("{text}\n").as_bytes()).unwrap();
        assert_eq!(writer.line.capacity(), 0);
        let html = writer.finish().unwrap();
        assert!(html.windows(text.len()).any(|span| span == text.as_bytes()));
    }

    #[test]
    fn short_ascii_html_cells_use_one_write_each() {
        struct Counting {
            writes: usize,
            bytes: Vec<u8>,
        }
        impl Write for Counting {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut output = Counting {
            writes: 0,
            bytes: Vec::new(),
        };
        write_plain_cells(&mut output, "first name\t42\t", false).unwrap();
        assert_eq!(output.writes, 3);
        assert_eq!(output.bytes, b"<td>first name</td><td>42</td><td></td>");

        let mut header = Counting {
            writes: 0,
            bytes: Vec::new(),
        };
        write_plain_cells(&mut header, &"x".repeat(256), true).unwrap();
        assert_eq!(header.writes, 1);
        assert!(header.bytes.starts_with(b"<th>"));
        assert!(header.bytes.ends_with(b"</th>"));
    }

    #[test]
    fn short_unicode_cells_use_single_frame_without_skipping_controls() {
        struct Counter {
            writes: usize,
            bytes: Vec<u8>,
        }
        impl Write for Counter {
            fn write(&mut self, input: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(input);
                Ok(input.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut counter = Counter {
            writes: 0,
            bytes: Vec::new(),
        };
        write_plain_cells(&mut counter, "café\t東京", false).unwrap();
        assert_eq!(counter.writes, 2);
        assert_eq!(counter.bytes, "<td>café</td><td>東京</td>".as_bytes());

        let mut escaped = Vec::new();
        write_plain_cells(&mut escaped, "a\u{0085}b", false).unwrap();
        assert_eq!(escaped, b"<td>a&#133;b</td>");
    }

    #[test]
    fn special_html_cells_keep_escaping_and_unicode() {
        let mut output = Vec::new();
        write_plain_cells(&mut output, "Tom & Sue\t<svg>\tcafé\tline\rbreak", false).unwrap();
        assert_eq!(
            output,
            "<td>Tom &amp; Sue</td><td>&lt;svg&gt;</td><td>café</td><td>line&#13;break</td>"
                .as_bytes()
        );
    }

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

#[cfg(test)]
mod medium_html_framing_tests {
    use super::*;

    #[derive(Default)]
    struct Counting {
        calls: usize,
        bytes: Vec<u8>,
    }

    impl Write for Counting {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn safe_one_to_four_kibibyte_cells_need_one_write() {
        for length in [257, 1024, 2048, 4096] {
            let mut output = Counting::default();
            write_plain_cells(&mut output, &"A".repeat(length), false).unwrap();
            assert_eq!(output.calls, 1, "length={length}");
            assert_eq!(output.bytes.len(), length + 9);
            assert!(output.bytes.starts_with(b"<td>"));
            assert!(output.bytes.ends_with(b"</td>"));
        }
        let mut output = Counting::default();
        let unicode = "é".repeat(2048);
        write_plain_cells(&mut output, &unicode, true).unwrap();
        assert_eq!(output.calls, 1);
        assert_eq!(&output.bytes[4..output.bytes.len() - 5], unicode.as_bytes());
    }

    #[test]
    fn long_cells_and_special_html_characters_keep_escaping() {
        let mut output = Vec::new();
        let input = format!("{}<&", "x".repeat(4096));
        write_plain_cells(&mut output, &input, false).unwrap();
        assert_eq!(
            output,
            format!("<td>{}&lt;&amp;</td>", "x".repeat(4096)).as_bytes()
        );
        let mut output = Vec::new();
        write_plain_cells(&mut output, &"z".repeat(4097), false).unwrap();
        assert_eq!(output.len(), 4097 + 9);
    }
}

#[cfg(test)]
mod html_escape_batching_tests {
    use super::*;

    #[derive(Default)]
    struct CountingSink {
        writes: usize,
        bytes: Vec<u8>,
    }
    impl Write for CountingSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn thousands_of_escaped_characters_are_batched_without_changing_html() {
        let mut sink = CountingSink::default();
        let input = "<>&\"'\n\r".repeat(2048);
        write_cell(&mut sink, &input).unwrap();
        let expected = "&lt;&gt;&amp;&quot;&#39;&#10;&#13;".repeat(2048);
        assert_eq!(sink.bytes, expected.as_bytes());
        assert!(
            sink.writes <= 32,
            "unexpected fragmented writes: {}",
            sink.writes
        );
    }

    #[test]
    fn long_plain_utf8_still_uses_one_write_and_mixed_cells_preserve_bytes() {
        let value = "café".repeat(2048);
        let mut plain = CountingSink::default();
        write_cell(&mut plain, &value).unwrap();
        assert_eq!(plain.writes, 1);
        assert_eq!(plain.bytes, value.as_bytes());

        let mut mixed = Vec::new();
        write_cell(&mut mixed, "é<&🍕\nZ").unwrap();
        assert_eq!(mixed, "é&lt;&amp;🍕&#10;Z".as_bytes());
    }
}
