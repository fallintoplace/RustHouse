use crate::tsv_escape::decode_tsv_field;
use std::borrow::Cow;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_SQL_VALUES_COLUMNS: usize = 64;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn append_literal(output: &mut Vec<u8>, field: &[u8]) -> io::Result<()> {
    if field == b"\\N" {
        output.extend_from_slice(b"NULL");
        return Ok(());
    }
    let decoded: Cow<'_, [u8]> = if field.contains(&b'\\') {
        Cow::Owned(decode_tsv_field(field)?)
    } else {
        Cow::Borrowed(field)
    };
    let value = std::str::from_utf8(&decoded)
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

fn encode_values_row_into(line: &[u8], columns: usize, values: &mut Vec<u8>) -> io::Result<()> {
    values.clear();
    values.reserve(
        line.len()
            .saturating_add(columns.saturating_mul(3))
            .saturating_add(2),
    );
    let mut encoded_field_count = 0;
    values.push(b'(');
    for field in line.split(|&byte| byte == b'\t') {
        if encoded_field_count > 0 {
            values.extend_from_slice(b", ");
        }
        append_literal(values, field)?;
        encoded_field_count += 1;
    }
    if encoded_field_count != columns {
        return Err(invalid("SQL VALUES row width differs from the header"));
    }
    values.push(b')');
    Ok(())
}

#[cfg(test)]
fn encode_values_row(line: &[u8], columns: usize) -> io::Result<Vec<u8>> {
    let mut encoded = Vec::new();
    encode_values_row_into(line, columns, &mut encoded)?;
    Ok(encoded)
}

fn emit_values_row<W: Write>(
    output: &mut W,
    encoded: &mut Vec<u8>,
    rows: &mut usize,
    line: &[u8],
    columns: usize,
) -> io::Result<()> {
    encode_values_row_into(line, columns, encoded)?;
    if *rows == 0 {
        output.write_all(b"VALUES ")?;
    } else {
        output.write_all(b",\n")?;
    }
    output.write_all(encoded)?;
    *rows += 1;
    Ok(())
}

pub struct SqlValuesWriter<W: Write> {
    output: W,
    columns: Option<usize>,
    line: Vec<u8>,
    encoded: Vec<u8>,
    rows: usize,
    finished: bool,
}

impl<W: Write> SqlValuesWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            columns: None,
            line: Vec::new(),
            encoded: Vec::new(),
            rows: 0,
            finished: false,
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        let Some(columns) = self.columns else {
            let columns = self.line.split(|&byte| byte == b'\t').count();
            if columns == 0 || columns > MAX_SQL_VALUES_COLUMNS {
                return Err(invalid("SQL VALUES supports 1 to 64 columns"));
            }
            self.columns = Some(columns);
            self.line.clear();
            return Ok(());
        };
        emit_values_row(&mut self.output, &mut self.encoded, &mut self.rows, &self.line, columns)?;
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
            if fragment.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
                return Err(invalid("SQL VALUES input line exceeds 1 MiB"));
            }
            if complete && self.line.is_empty() {
                if let Some(columns) = self.columns {
                    emit_values_row(
                        &mut self.output,
                        &mut self.encoded,
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

    #[test]
    fn contiguous_sql_rows_do_not_use_input_line_storage() {
        let mut writer = SqlValuesWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        let before = writer.line.capacity();
        let value = "a".repeat(16 * 1024);
        writer.write_all(format!("{value}\n").as_bytes()).unwrap();
        assert_eq!(writer.line.capacity(), before);
        writer.finish().unwrap();
        assert!(writer.output.ends_with(format!("('{value}');\n").as_bytes()));
    }

    #[test]
    fn bulk_input_serializes_multiple_rows_and_fragmented_tail() {
        let mut writer = SqlValuesWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        writer.write_all("7\n".repeat(1000).as_bytes()).unwrap();
        writer.write_all(b"par").unwrap();
        writer.write_all(b"tial\n").unwrap();
        writer.finish().unwrap();
        assert_eq!(writer.rows, 1001);
        assert!(writer.output.starts_with(b"VALUES ('7'),\n('7')"));
        assert!(writer.output.ends_with(b"('partial');\n"));
    }

    #[test]
    fn repeated_finish_does_not_duplicate_sql_or_accept_more_rows() {
        let mut writer = SqlValuesWriter::new(Vec::new());
        writer.write_all(b"value\n1\n").unwrap();
        writer.finish().unwrap();
        writer.finish().unwrap();
        assert_eq!(writer.output, b"VALUES ('1');\n");
        assert_eq!(
            writer.write_all(b"2\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn quoted_text_escapes_apostrophes_and_backslashes() {
        let mut bytes = Vec::new();
        append_quoted_text(&mut bytes, "a'b\\c");
        assert_eq!(bytes, b"'a''b\\\\c'");
    }

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
    fn encodes_large_plain_utf8_literals_without_changing_content() {
        let field = "café".repeat(4096);
        let mut output = Vec::new();
        append_literal(&mut output, field.as_bytes()).unwrap();
        assert_eq!(output.len(), field.len() + 2);
        assert_eq!(&output[1..1 + field.len()], field.as_bytes());
        assert_eq!(output[0], b'\'');
        assert_eq!(*output.last().unwrap(), b'\'');
    }

    #[test]
    fn decodes_hex_escapes_with_shared_decoder() {
        assert_eq!(
            render(b"value\nx\\x41\\x42\n").unwrap(),
            "VALUES ('xAB');\n"
        );
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
    fn row_capacity_covers_plain_large_values() {
        let row = vec![b'a'; 16 * 1024];
        let encoded = encode_values_row(&row, 1).unwrap();
        assert_eq!(encoded.len(), row.len() + 4);
        assert_eq!(encoded[0], b'(');
        assert_eq!(*encoded.last().unwrap(), b')');
    }

    #[test]
    fn row_encoder_rejects_wrong_width_before_streaming() {
        assert!(encode_values_row(b"one", 2).is_err());
        assert_eq!(encode_values_row(b"1\t\\N", 2).unwrap(), b"('1', NULL)");
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
mod reuse_sql_values_row_buffer_tests {
    use super::*;

    #[test]
    fn encoded_buffer_capacity_is_reused_for_subsequent_rows() {
        let mut writer = SqlValuesWriter::new(Vec::new());
        let large = "x".repeat(32 * 1024);
        writer.write_all(format!("value\n{large}\n").as_bytes()).unwrap();
        let capacity = writer.encoded.capacity();
        writer.write_all(b"small\nsmall\n").unwrap();
        assert_eq!(writer.encoded.capacity(), capacity);
        writer.finish().unwrap();
        assert!(writer.output.ends_with(b"('small'),\n('small');\n"));
    }
}
