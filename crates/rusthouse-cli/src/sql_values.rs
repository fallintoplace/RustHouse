use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn decode_field(field: &[u8]) -> io::Result<Vec<u8>> {
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
        output.push(match escaped {
            b'\\' => b'\\',
            b't' => b'\t',
            b'n' => b'\n',
            b'r' => b'\r',
            b'0' => 0,
            b'x' => {
                let high = field
                    .get(index + 1)
                    .and_then(|byte| hex(*byte))
                    .ok_or_else(|| invalid("invalid TSV hex escape"))?;
                let low = field
                    .get(index + 2)
                    .and_then(|byte| hex(*byte))
                    .ok_or_else(|| invalid("invalid TSV hex escape"))?;
                index += 2;
                (high << 4) | low
            }
            _ => return Err(invalid("unknown TSV escape")),
        });
        index += 1;
    }
    Ok(output)
}

fn append_literal(output: &mut Vec<u8>, field: &[u8]) -> io::Result<()> {
    if field == b"\\N" {
        output.extend_from_slice(b"NULL");
        return Ok(());
    }
    let decoded = decode_field(field)?;
    let value = std::str::from_utf8(&decoded)
        .map_err(|_| invalid("SQL VALUES output requires UTF-8 text"))?;
    output.push(b'\'');
    for ch in value.chars() {
        match ch {
            '\'' => output.extend_from_slice(b"''"),
            '\\' => output.extend_from_slice(b"\\\\"),
            other => {
                let mut bytes = [0; 4];
                output.extend_from_slice(other.encode_utf8(&mut bytes).as_bytes());
            }
        }
    }
    output.push(b'\'');
    Ok(())
}

pub struct SqlValuesWriter<W: Write> {
    output: W,
    columns: Option<usize>,
    line: Vec<u8>,
    rows: usize,
}

impl<W: Write> SqlValuesWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            columns: None,
            line: Vec::new(),
            rows: 0,
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        let fields = self.line.split(|&byte| byte == b'\t');
        if self.columns.is_none() {
            let columns = fields.count();
            if columns == 0 || columns > 64 {
                return Err(invalid("SQL VALUES supports 1 to 64 columns"));
            }
            self.columns = Some(columns);
            self.line.clear();
            return Ok(());
        }
        let mut values = Vec::new();
        let mut count = 0;
        values.push(b'(');
        for field in fields {
            if count > 0 {
                values.extend_from_slice(b", ");
            }
            append_literal(&mut values, field)?;
            count += 1;
        }
        if Some(count) != self.columns {
            return Err(invalid("SQL VALUES row width differs from the header"));
        }
        values.push(b')');
        if self.rows == 0 {
            self.output.write_all(b"VALUES ")?;
        } else {
            self.output.write_all(b",\n")?;
        }
        self.output.write_all(&values)?;
        self.rows += 1;
        self.line.clear();
        Ok(())
    }

    pub fn finish(&mut self) -> io::Result<()> {
        if !self.line.is_empty() {
            return Err(invalid("SQL VALUES input has an unterminated line"));
        }
        if self.rows > 0 {
            self.output.write_all(b";\n")?;
        }
        self.output.flush()
    }
}

impl<W: Write> Write for SqlValuesWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_line()?;
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(invalid("SQL VALUES input line exceeds 1 MiB"));
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
    fn rejects_invalid_tsv_and_wide_rows() {
        assert!(render(b"id\nbad\\q\n").is_err());
        assert!(render(b"id\nbad\\xg0\n").is_err());
        assert!(render(b"a\tb\n1\n").is_err());
        assert!(render(b"id\n1").is_err());
    }
}
