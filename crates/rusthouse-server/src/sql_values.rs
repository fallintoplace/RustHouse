use rusthouse_format::decode_tsv_field;
use std::borrow::Cow;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

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
    Ok(())
}

fn encode_values_row(line: &[u8], columns: usize, values: &mut Vec<u8>) -> io::Result<()> {
    values.clear();
    values.reserve(
        line.len()
            .saturating_add(columns.saturating_mul(3))
            .saturating_add(2),
    );
    let mut count = 0;
    values.push(b'(');
    for field in line.split(|&byte| byte == b'\t') {
        if count > 0 {
            values.extend_from_slice(b", ");
        }
        append_literal(values, field)?;
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
    rows: &mut usize,
    line: &[u8],
    columns: usize,
) -> io::Result<()> {
    encode_values_row(line, columns, values)?;
    if *rows == 0 {
        output.write_all(b"VALUES ")?;
    } else {
        output.write_all(b",\n")?;
    }
    output.write_all(values)?;
    *rows += 1;
    Ok(())
}

pub struct SqlValuesWriter<W: Write> {
    output: W,
    columns: Option<usize>,
    line: Vec<u8>,
    values: Vec<u8>,
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
            rows: 0,
            finished: false,
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        let Some(columns) = self.columns else {
            let columns = self.line.split(|&byte| byte == b'\t').count();
            if columns == 0 || columns > 64 {
                return Err(invalid("SQL VALUES supports 1 to 64 columns"));
            }
            self.columns = Some(columns);
            self.line.clear();
            return Ok(());
        };
        emit_values_row(&mut self.output, &mut self.values, &mut self.rows, &self.line, columns)?;
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
                        &mut self.values,
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
        append_literal(&mut output, field.as_bytes()).unwrap();
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
