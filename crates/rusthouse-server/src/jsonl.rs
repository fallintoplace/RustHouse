//! Convert bounded TSV query results into newline-delimited JSON objects.
//! Numeric values remain strings to preserve Int128 and UInt128 precision.
use super::write_json_string;
use std::collections::HashSet;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn decode_tsv_value(value: &[u8]) -> io::Result<String> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        if value[index] != b'\\' {
            decoded.push(value[index]);
            index += 1;
            continue;
        }
        index += 1;
        let escaped = *value
            .get(index)
            .ok_or_else(|| invalid("incomplete TSV escape"))?;
        match escaped {
            b'\\' => decoded.push(b'\\'),
            b't' => decoded.push(b'\t'),
            b'n' => decoded.push(b'\n'),
            b'r' => decoded.push(b'\r'),
            b'0' => decoded.push(0),
            b'x' => {
                let high = *value
                    .get(index + 1)
                    .ok_or_else(|| invalid("short TSV hex escape"))?;
                let low = *value
                    .get(index + 2)
                    .ok_or_else(|| invalid("short TSV hex escape"))?;
                let digit = |byte: u8| (byte as char).to_digit(16).map(|value| value as u8);
                let high = digit(high).ok_or_else(|| invalid("invalid TSV hex escape"))?;
                let low = digit(low).ok_or_else(|| invalid("invalid TSV hex escape"))?;
                decoded.push((high << 4) | low);
                index += 2;
            }
            _ => return Err(invalid("unknown TSV escape")),
        }
        index += 1;
    }
    String::from_utf8(decoded).map_err(|_| invalid("JSONL field is not UTF-8"))
}

fn parse_headers(line: &[u8]) -> io::Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut headers = Vec::new();
    for header in line.split(|byte| *byte == b'\t') {
        let name = std::str::from_utf8(header)
            .map_err(|_| invalid("JSONL header is not UTF-8"))?;
        if !seen.insert(name) {
            return Err(invalid("JSONL needs unique projected column names"));
        }
        headers.push(name.to_owned());
    }
    Ok(headers)
}

fn write_json_row<W: Write>(output: &mut W, headers: &[String], line: &[u8]) -> io::Result<()> {
    if 1 + line.iter().filter(|byte| **byte == b'\t').count() != headers.len() {
        return Err(invalid("JSONL row does not match header column count"));
    }
    output.write_all(b"{")?;
    for (index, (header, field)) in headers
        .iter()
        .zip(line.split(|byte| *byte == b'\t'))
        .enumerate()
    {
        if index != 0 {
            output.write_all(b",")?;
        }
        write_json_string(output, header)?;
        output.write_all(b":")?;
        if field == b"\\N" {
            output.write_all(b"null")?;
        } else {
            let text = decode_tsv_value(field)?;
            write_json_string(output, &text)?;
        }
    }
    output.write_all(b"}\n")
}

pub struct JsonlWriter<W: Write> {
    output: W,
    headers: Option<Vec<String>>,
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
        if self.headers.is_none() {
            let headers = parse_headers(&self.line);
            self.line.clear();
            self.headers = Some(headers?);
            return Ok(());
        }

        let result = write_json_row(&mut self.output, self.headers.as_ref().unwrap(), &self.line);
        self.line.clear();
        result
    }
}

impl<W: Write> Write for JsonlWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_line()?;
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(invalid("JSONL row exceeds 1 MiB"));
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
    fn jsonl_phases_validate_headers_before_writing_rows() {
        assert_eq!(
            parse_headers(b"left\tright").unwrap(),
            vec!["left".to_owned(), "right".to_owned()]
        );
        assert_eq!(
            parse_headers(b"left\tleft").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut output = Vec::new();
        write_json_row(&mut output, &["key".to_owned()], b"hello\\nworld").unwrap();
        assert_eq!(output, b"{\"key\":\"hello\\nworld\"}\n");
    }

    #[test]
    fn decodes_sql_tsv_escaping_and_preserves_large_numbers() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"label\tamount\tmissing\n").unwrap();
        writer
            .write_all(b"hello\\tworld\t340282366920938463463374607431768211455\t\\N\n")
            .unwrap();
        writer.write_all(b"back\\\\slash\tNULL\t\\N\n").unwrap();
        let output = String::from_utf8(writer.output).unwrap();
        assert_eq!(
            output,
            "{\"label\":\"hello\\tworld\",\"amount\":\"340282366920938463463374607431768211455\",\"missing\":null}\n{\"label\":\"back\\\\slash\",\"amount\":\"NULL\",\"missing\":null}\n"
        );
    }

    #[test]
    fn rejects_duplicate_columns_malformed_rows_and_invalid_escapes() {
        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(b"a\ta\n").is_err());

        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"a\tb\n").unwrap();
        assert!(writer.write_all(b"1\n").is_err());

        for invalid in [&b"bad\\q"[..], &b"bad\\xG0"[..], &b"bad\\x"[..]] {
            assert!(decode_tsv_value(invalid).is_err());
        }
        assert_eq!(
            decode_tsv_value(b"quoted\\x22word").unwrap(),
            "quoted\"word"
        );
    }

    #[test]
    fn handles_fragmented_writes_and_bounded_lines() {
        let mut writer = JsonlWriter::new(Vec::new());
        writer.write_all(b"key").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"\na").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"bc\n").unwrap();
        assert_eq!(
            String::from_utf8(writer.output).unwrap(),
            "{\"key\":\"abc\"}\n"
        );

        let mut writer = JsonlWriter::new(Vec::new());
        assert!(writer.write_all(&vec![b'x'; MAX_LINE_BYTES + 1]).is_err());
    }
}
