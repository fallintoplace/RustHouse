use rusthouse_format::decode_tsv_field_into;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn append_literal(output: &mut Vec<u8>, field: &[u8], decoded: &mut Vec<u8>) -> io::Result<()> {
    if field == b"\\N" {
        output.extend_from_slice(b"NULL");
        return Ok(());
    }
    let bytes: &[u8] = if field.contains(&b'\\') {
        decode_tsv_field_into(field, decoded)?;
        decoded.as_slice()
    } else {
        field
    };
    let value = std::str::from_utf8(bytes)
        .map_err(|_| invalid("SQL VALUES output requires UTF-8 text"))?;
    append_quoted_text(output, value);
    Ok(())
}

fn append_quoted_text(output: &mut Vec<u8>, value: &str) {
    output.push(b'\'');
    let bytes = value.as_bytes();
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'\'' || byte == b'\\' {
            output.extend_from_slice(&bytes[start..index]);
            if byte == b'\'' {
                output.extend_from_slice(b"''");
            } else {
                output.extend_from_slice(b"\\\\");
            }
            start = index + 1;
        }
    }
    output.extend_from_slice(&bytes[start..]);
    output.push(b'\'');
}

#[cfg(test)]
fn encode_values_row(line: &[u8], columns: usize, values: &mut Vec<u8>) -> io::Result<()> {
    encode_values_row_prefixed(line, columns, values, b"", &mut Vec::new())
}

fn encode_values_row_prefixed(
    line: &[u8], columns: usize, values: &mut Vec<u8>, prefix: &[u8],
    decoded: &mut Vec<u8>,
) -> io::Result<()> {
    values.clear();
    values.reserve(
        line.len()
            .saturating_add(columns.saturating_mul(3))
            .saturating_add(prefix.len())
            .saturating_add(2),
    );
    values.extend_from_slice(prefix);
    let mut count = 0;
    values.push(b'(');
    for field in line.split(|&byte| byte == b'\t') {
        if count >= columns {
            return Err(invalid("SQL VALUES row width differs from the header"));
        }
        if count > 0 {
            values.extend_from_slice(b", ");
        }
        append_literal(values, field, decoded)?;
        count += 1;
    }
    if count != columns {
        return Err(invalid("SQL VALUES row width differs from the header"));
    }
    values.push(b')');
    Ok(())
}

fn emit_values_row<W: Write>(
    output: &mut W,
    values: &mut Vec<u8>,
    decoded: &mut Vec<u8>,
    rows: &mut usize,
    line: &[u8],
    columns: usize,
) -> io::Result<()> {
    let prefix: &[u8] = if *rows == 0 { b"VALUES " } else { b",\n" };
    encode_values_row_prefixed(line, columns, values, prefix, decoded)?;
    output.write_all(values)?;
    *rows += 1;
    Ok(())
}

pub struct SqlValuesWriter<W: Write> {
    output: W,
    columns: Option<usize>,
    line: Vec<u8>,
    values: Vec<u8>,
    decoded: Vec<u8>,
    rows: usize,
    finished: bool,
}

impl<W: Write> SqlValuesWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            columns: None,
            line: Vec::new(),
            values: Vec::new(),
            decoded: Vec::new(),
            rows: 0,
            finished: false,
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        // A carriage return immediately before LF is a record delimiter,
        // not a byte of the last TSV field. This also handles split writes.
        if self.line.last() == Some(&b'\r') {
            self.line.pop();
        }
        let Some(columns) = self.columns else {
            let columns = self.line.split(|&byte| byte == b'\t').count();
            if columns == 0 || columns > 64 {
                return Err(invalid("SQL VALUES supports 1 to 64 columns"));
            }
            self.columns = Some(columns);
            self.line.clear();
            return Ok(());
        };
        emit_values_row(&mut self.output, &mut self.values, &mut self.decoded, &mut self.rows, &self.line, columns)?;
        self.line.clear();
        Ok(())
    }

    pub fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return self.output.flush();
        }
        if !self.line.is_empty() {
            return Err(invalid("SQL VALUES input has an unterminated line"));
        }
        if self.rows > 0 {
            self.output.write_all(b";\n")?;
        }
        self.finished = true;
        self.output.flush()
    }
}

impl<W: Write> Write for SqlValuesWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.finished {
            return Err(invalid("SQL VALUES output is already finished"));
        }
        for chunk in bytes.split_inclusive(|&byte| byte == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            let fragment = if complete {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            // Keep the direct-row fast path safe for CRLF input as well.
            let fragment = if complete && self.line.is_empty() {
                fragment.strip_suffix(b"\r").unwrap_or(fragment)
            } else {
                fragment
            };
            if fragment.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
                return Err(invalid("SQL VALUES input line exceeds 1 MiB"));
            }
            if complete && self.line.is_empty() {
                if let Some(columns) = self.columns {
                    emit_values_row(
                        &mut self.output,
                        &mut self.values,
                        &mut self.decoded,
                        &mut self.rows,
                        fragment,
                        columns,
                    )?;
                    continue;
                }
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

    fn render(input: &[u8]) -> io::Result<String> {
        let mut output = Vec::new();
        {
            let mut writer = SqlValuesWriter::new(&mut output);
            for chunk in input.chunks(3) {
                writer.write_all(chunk)?;
            }
            writer.finish()?;
        }
        String::from_utf8(output).map_err(|_| invalid("invalid output UTF-8"))
    }

    #[test]
    fn direct_large_rows_skip_input_staging() {
        let mut writer = SqlValuesWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        let before = writer.line.capacity();
        let value = "z".repeat(16 * 1024);
        writer.write_all(format!("{value}\n").as_bytes()).unwrap();
        assert_eq!(writer.line.capacity(), before);
        writer.finish().unwrap();
        assert!(writer.output.ends_with(format!("('{value}');\n").as_bytes()));
    }

    #[test]
    fn sql_values_reuses_decoding_storage_between_escaped_rows() {
        let mut writer = SqlValuesWriter::new(Vec::new());
        writer.write_all(b"note\nline\\t1\n").unwrap();
        let first_buffer = writer.decoded.as_ptr();
        let first_capacity = writer.decoded.capacity();
        assert!(first_capacity >= 6);
        writer.write_all(b"line\\t2\n").unwrap();
        assert_eq!(writer.decoded.as_ptr(), first_buffer);
        assert_eq!(writer.decoded.capacity(), first_capacity);
        writer.finish().unwrap();
        assert_eq!(writer.output, b"VALUES ('line\t1'),\n('line\t2');\n");
    }

    #[test]
    fn uses_shared_tsv_decoder_for_hex_and_control_escapes() {
        assert_eq!(
            render(b"value\ncaf\\xC3\\xA9\n").unwrap(),
            "VALUES ('café');\n"
        );
        assert_eq!(
            render(b"value\n\\b\\f\\v\n").unwrap().as_bytes(),
            b"VALUES ('\x08\x0c\x0b');\n"
        );
        assert!(render(b"value\n\\xC3\n").is_err());
    }

    #[test]
    fn keeps_large_unescaped_utf8_fields_byte_for_byte() {
        let field = "café".repeat(4096);
        let mut output = Vec::new();
        append_literal(&mut output, field.as_bytes(), &mut Vec::new()).unwrap();
        assert_eq!(output.len(), field.len() + 2);
        assert_eq!(&output[1..1 + field.len()], field.as_bytes());
        assert_eq!(output[0], b'\'');
        assert_eq!(*output.last().unwrap(), b'\'');
    }

    #[test]
    fn outputs_replayable_values_and_escapes() {
        assert_eq!(
            render(b"id\tname\tnote\n1\tO'Brien\t\\N\n2\tpath\\\\folder\t\n").unwrap(),
            "VALUES ('1', 'O''Brien', NULL),\n('2', 'path\\\\folder', '');\n"
        );
        assert_eq!(render(b"id\tname\n").unwrap(), "");
        assert_eq!(
            render(b"one\ttwo\n\\N\t\\N\n").unwrap(),
            "VALUES (NULL, NULL);\n"
        );
    }

    #[test]
    fn sql_values_reuses_row_buffer_for_many_rows() {
        let mut writer = SqlValuesWriter::new(Vec::new());
        writer.write_all(b"id\n1\n2\n3\n").unwrap();
        assert!(writer.values.capacity() >= 5);
        writer.finish().unwrap();
        assert_eq!(writer.output, b"VALUES ('1'),\n('2'),\n('3');\n");
    }

    #[test]
    fn row_encoder_rejects_wrong_width_before_streaming() {
        let mut values = Vec::new();
        assert!(encode_values_row(b"one", 2, &mut values).is_err());
        encode_values_row(b"1\t\\N", 2, &mut values).unwrap();
        assert_eq!(values, b"('1', NULL)");
    }

    #[test]
    fn finishing_twice_does_not_append_another_terminator() {
        let mut writer = SqlValuesWriter::new(Vec::new());
        writer.write_all(b"id\n1\n").unwrap();
        writer.finish().unwrap();
        writer.finish().unwrap();
        assert_eq!(writer.output, b"VALUES ('1');\n");
        assert_eq!(
            writer.write_all(b"2\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn rejects_invalid_tsv_and_wide_rows() {
        assert!(render(b"id\nbad\\q\n").is_err());
        assert!(render(b"id\nbad\\xg0\n").is_err());
        assert!(render(b"a\tb\n1\n").is_err());
        assert!(render(b"id\n1").is_err());
    }
}

#[cfg(test)]
mod overwide_values_tests {
    use super::*;

    #[test]
    fn extra_fields_do_not_write_partial_values_output() {
        let mut values = Vec::new();
        assert!(encode_values_row(b"first\tsecond", 1, &mut values).is_err());
        encode_values_row(b"first", 1, &mut values).unwrap();
        assert_eq!(values, b"('first')");
    }
}

#[cfg(test)]
mod crlf_values_regressions {
    use super::*;
    #[test]
    fn strips_record_carriage_returns_not_escaped_field_data() {
        let mut writer = SqlValuesWriter::new(Vec::new());
        writer.write_all(b"id\tvalue\r\n").unwrap();
        writer.write_all(b"1\tfirst\r\n2\tsecond\n3\tthird\\r\r\n").unwrap();
        writer.finish().unwrap();
        assert_eq!(writer.output,
            b"VALUES ('1', 'first'),\n('2', 'second'),\n('3', 'third\r');\n");
    }
    #[test]
    fn handles_crlf_split_across_write_boundaries() {
        let mut writer = SqlValuesWriter::new(Vec::new());
        for part in [b"name\r".as_slice(), b"\nvalue\r", b"\n"] {
            writer.write_all(part).unwrap();
        }
        writer.finish().unwrap();
        assert_eq!(writer.output, b"VALUES ('value');\n");
    }
}

#[cfg(test)]
mod single_write_server_values_row_tests {
    use super::*;

    #[derive(Default)]
    struct Counted { calls: usize, bytes: Vec<u8> }
    impl Write for Counted {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[test]
    fn prefixed_rows_are_written_once_without_changing_sql() {
        let mut output = Counted::default();
        let mut encoded = Vec::new();
        let mut decoded = Vec::new();
        let mut rows = 0;
        emit_values_row(&mut output, &mut encoded, &mut decoded, &mut rows, b"one", 1).unwrap();
        emit_values_row(&mut output, &mut encoded, &mut decoded, &mut rows, b"two", 1).unwrap();
        assert_eq!(rows, 2);
        assert_eq!(output.calls, 2);
        assert_eq!(output.bytes, b"VALUES ('one'),\n('two')");
        encode_values_row(b"three", 1, &mut encoded).unwrap();
        assert_eq!(encoded, b"('three')");
    }
}
