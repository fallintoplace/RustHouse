//! Convert the SQL executor's TSV stream into a bounded, escaped HTML table.
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn decode_tsv_field(field: &[u8]) -> io::Result<Vec<u8>> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidData, message);
    let mut output = Vec::with_capacity(field.len());
    let mut index = 0;
    while index < field.len() {
        if field[index] != b'\\' {
            output.push(field[index]);
            index += 1;
            continue;
        }
        index += 1;
        let escaped = *field
            .get(index)
            .ok_or_else(|| invalid("incomplete TSV escape"))?;
        match escaped {
            b'\\' => output.push(b'\\'),
            b't' => output.push(b'\t'),
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b'0' => output.push(0),
            b'x' => {
                let high = *field
                    .get(index + 1)
                    .ok_or_else(|| invalid("short TSV hex escape"))?;
                let low = *field
                    .get(index + 2)
                    .ok_or_else(|| invalid("short TSV hex escape"))?;
                let high = hex_digit(high).ok_or_else(|| invalid("invalid TSV hex escape"))?;
                let low = hex_digit(low).ok_or_else(|| invalid("invalid TSV hex escape"))?;
                output.push((high << 4) | low);
                index += 2;
            }
            _ => return Err(invalid("unknown TSV escape")),
        }
        index += 1;
    }
    Ok(output)
}

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
        let count = 1 + self.line.iter().filter(|&&byte| byte == b'\t').count();
        let header = self.columns.is_none();
        if let Some(columns) = self.columns {
            if columns != count {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTML row does not match header column count",
                ));
            }
        }

        let fields = text
            .split('\t')
            .map(|field| {
                if header || field == "\\N" {
                    return Ok(field.to_owned());
                }
                String::from_utf8(decode_tsv_field(field.as_bytes())?).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "HTML field is not valid UTF-8")
                })
            })
            .collect::<io::Result<Vec<_>>>()?;

        if header {
            self.output.write_all(b"<table>\n<thead><tr>")?;
        } else {
            self.output.write_all(b"<tr>")?;
        }
        write_cells(&mut self.output, &fields, header)?;
        if header {
            self.output.write_all(b"</tr></thead>\n<tbody>\n")?;
            self.columns = Some(count);
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

fn write_cells<W: Write>(output: &mut W, fields: &[String], header: bool) -> io::Result<()> {
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

fn write_cell<W: Write>(output: &mut W, field: &str) -> io::Result<()> {
    for ch in field.chars() {
        match ch {
            '&' => output.write_all(b"&amp;")?,
            '<' => output.write_all(b"&lt;")?,
            '>' => output.write_all(b"&gt;")?,
            '"' => output.write_all(b"&quot;")?,
            '\'' => output.write_all(b"&#39;")?,
            ch if ch.is_control() => write!(output, "&#{};", ch as u32)?,
            other => output.write_all(other.encode_utf8(&mut [0u8; 4]).as_bytes())?,
        }
    }
    Ok(())
}

impl<W: Write> Write for HtmlWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_line()?;
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "HTML row exceeds 1 MiB",
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
