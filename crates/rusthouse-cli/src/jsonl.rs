use std::collections::HashSet;
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

    pub fn into_inner(self) -> W {
        self.output
    }

    fn finish_line(&mut self) -> io::Result<()> {
        if self.headers.is_none() {
            let headers = parse_headers(&self.line);
            self.line.clear();
            self.headers = Some(headers?);
            return Ok(());
        }

        let headers = self.headers.as_ref().unwrap();
        let result = write_json_row(&mut self.output, headers, &self.line);
        self.line.clear();
        result
    }
}

fn parse_headers(line: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let mut seen = HashSet::new();
    let mut headers = Vec::new();
    for header in line.split(|byte| *byte == b'\t') {
        if !seen.insert(header) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSONL requires unique projected column names",
            ));
        }
        headers.push(header.to_vec());
    }
    Ok(headers)
}

fn write_json_row<W: Write>(output: &mut W, headers: &[Vec<u8>], line: &[u8]) -> io::Result<()> {
    let field_count = 1 + line.iter().filter(|byte| **byte == b'\t').count();
    if field_count != headers.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "JSONL row does not match header column count",
        ));
    }

    output.write_all(b"{")?;
    for (index, (header, value)) in headers
        .iter()
        .zip(line.split(|byte| *byte == b'\t'))
        .enumerate()
    {
        if index > 0 {
            output.write_all(b",")?;
        }
        write_json_string(output, header)?;
        output.write_all(b":")?;
        if value == b"NULL" || value == b"\\N" {
            output.write_all(b"null")?;
        } else {
            let decoded = decode_tsv_field(value)?;
            write_json_string(output, &decoded)?;
        }
    }
    output.write_all(b"}\n")
}

fn write_json_string<W: Write>(writer: &mut W, input: &[u8]) -> io::Result<()> {
    let input = std::str::from_utf8(input).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "JSONL field is not valid UTF-8")
    })?;
    writer.write_all(b"\"")?;
    let mut start = 0;
    for (index, ch) in input.char_indices() {
        let escaped: Option<&[u8]> = match ch {
            '"' => Some(b"\\\""),
            '\\' => Some(b"\\\\"),
            '\n' => Some(b"\\n"),
            '\r' => Some(b"\\r"),
            '\t' => Some(b"\\t"),
            _ => None,
        };
        if let Some(escaped) = escaped {
            writer.write_all(&input.as_bytes()[start..index])?;
            writer.write_all(escaped)?;
            start = index + ch.len_utf8();
        } else if ch.is_control() {
            writer.write_all(&input.as_bytes()[start..index])?;
            write!(writer, "\\u{:04x}", ch as u32)?;
            start = index + ch.len_utf8();
        }
    }
    writer.write_all(&input.as_bytes()[start..])?;
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
    fn writes_plain_json_fields_in_bulk_without_losing_unicode() {
        struct CountingWriter {
            calls: usize,
            bytes: Vec<u8>,
        }
        impl Write for CountingWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut output = CountingWriter {
            calls: 0,
            bytes: Vec::new(),
        };
        write_json_string(&mut output, "plain café".as_bytes()).unwrap();
        assert_eq!(output.calls, 3);
        assert_eq!(output.bytes, "\"plain café\"".as_bytes());
    }

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

    #[test]
    fn rejects_invalid_utf8_in_headers_and_values() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"\xff\n").unwrap();
        assert_eq!(
            writer.write_all(b"1\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"name\n").unwrap();
        assert_eq!(
            writer.write_all(b"\xff\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[test]
    fn renders_typed_null_markers_as_json_null() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"message\tamount\n\\N\t1\n").unwrap();
        assert_eq!(
            writer.into_inner(),
            b"{\"message\":null,\"amount\":\"1\"}\n"
        );
    }

    #[test]
    fn formats_wide_rows_without_cloning_each_field() {
        let header = (0..32)
            .map(|index| format!("field{index}"))
            .collect::<Vec<_>>()
            .join("\t");
        let row = (0..32)
            .map(|index| format!("value{index}"))
            .collect::<Vec<_>>()
            .join("\t");
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(header.as_bytes()).unwrap();
        writer.write_all(b"\n").unwrap();
        for _ in 0..2 {
            writer.write_all(row.as_bytes()).unwrap();
            writer.write_all(b"\n").unwrap();
        }
        let output = String::from_utf8(writer.into_inner()).unwrap();
        assert_eq!(output.lines().count(), 2);
        assert_eq!(output.matches("\"field31\":\"value31\"").count(), 2);
        assert!(output.starts_with("{\"field0\":\"value0\","));
    }
}

#[cfg(test)]
mod decoded_tsv_json_regressions {
    use super::*;

    #[test]
    fn restores_escaped_sql_strings_before_json_quoting() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer
            .write_all(b"value\nline\\nnext\\tcell\\\\end\n")
            .unwrap();
        assert_eq!(
            writer.into_inner(),
            b"{\"value\":\"line\\nnext\\tcell\\\\end\"}\n"
        );
    }

    #[test]
    fn rejects_bad_tsv_escapes_instead_of_exporting_changed_data() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        assert_eq!(
            writer.write_all(b"bad\\q\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
