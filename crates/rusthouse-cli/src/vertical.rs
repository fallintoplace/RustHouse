use crate::tsv_escape::decode_tsv_field;
use std::borrow::Cow;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Format tab-separated SQL output one row at a time for terminal inspection.
///
/// The first input line contains column headers and is not printed until a data
/// row arrives. Header-only results produce no output.
pub struct VerticalWriter<W: Write> {
    output: W,
    headers: Option<Vec<Vec<u8>>>,
    line: Vec<u8>,
    rows: usize,
}

fn write_terminal_safe<W: Write>(output: &mut W, bytes: &[u8]) -> io::Result<()> {
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte.is_ascii_control() {
            output.write_all(&bytes[start..index])?;
            write!(output, "\\x{byte:02X}")?;
            start = index + 1;
        }
    }
    output.write_all(&bytes[start..])
}

fn decode_row_fields(line: &[u8]) -> io::Result<Vec<Cow<'_, [u8]>>> {
    line.split(|&byte| byte == b'\t')
        .map(|field| {
            if field == b"\\N" || !field.contains(&b'\\') {
                Ok(Cow::Borrowed(field))
            } else {
                decode_tsv_field(field).map(Cow::Owned)
            }
        })
        .collect()
}

fn validate_row_width(headers: &[Vec<u8>], line: &[u8]) -> io::Result<()> {
    let field_count = 1 + line.iter().filter(|&&byte| byte == b'\t').count();
    if field_count != headers.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "vertical row does not match header column count",
        ));
    }
    Ok(())
}

fn write_row<W: Write>(
    output: &mut W,
    row: usize,
    headers: &[Vec<u8>],
    line: &[u8],
) -> io::Result<()> {
    if !line.contains(&b'\\') {
        writeln!(output, "Row {row}:")?;
        for (header, field) in headers.iter().zip(line.split(|&byte| byte == b'\t')) {
            output.write_all(b"  ")?;
            write_terminal_safe(output, header)?;
            output.write_all(b": ")?;
            write_terminal_safe(output, field)?;
            output.write_all(b"\n")?;
        }
        return output.write_all(b"\n");
    }
    let fields = decode_row_fields(line)?;
    writeln!(output, "Row {row}:")?;
    for (header, field) in headers.iter().zip(&fields) {
        output.write_all(b"  ")?;
        write_terminal_safe(output, header)?;
        output.write_all(b": ")?;
        write_terminal_safe(output, field)?;
        output.write_all(b"\n")?;
    }
    output.write_all(b"\n")
}

impl<W: Write> VerticalWriter<W> {
    pub fn new(output: W) -> Self {
        Self {
            output,
            headers: None,
            line: Vec::new(),
            rows: 0,
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        if self.headers.is_none() {
            self.headers = Some(
                self.line
                    .split(|&byte| byte == b'\t')
                    .map(|header| header.to_vec())
                    .collect(),
            );
            self.line.clear();
            return Ok(());
        }

        let headers = self.headers.as_ref().unwrap();
        if let Err(error) = validate_row_width(headers, &self.line) {
            self.line.clear();
            return Err(error);
        }

        let row = self
            .rows
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "too many vertical rows"))?;
        write_row(&mut self.output, row, headers, &self.line)?;
        self.rows = row;
        self.line.clear();
        Ok(())
    }
}

impl<W: Write> Write for VerticalWriter<W> {
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
                    "vertical row exceeds 1 MiB",
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
    fn plain_rows_keep_controls_and_unicode_without_field_collection() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all("title\ncafé\n".as_bytes()).unwrap();
        assert_eq!(writer.output, "Row 1:\n  title: café\n\n".as_bytes());
    }

    #[test]
    fn processes_many_rows_from_one_bulk_write() {
        let mut writer = VerticalWriter::new(Vec::new());
        let input = format!("value\n{}", "42\n".repeat(1_000));
        writer.write_all(input.as_bytes()).unwrap();
        assert_eq!(writer.rows, 1_000);
        assert_eq!(
            String::from_utf8(writer.output)
                .unwrap()
                .matches("Row ")
                .count(),
            1_000
        );
    }

    #[test]
    fn decoded_row_fields_preserve_null_sentinels() {
        assert_eq!(
            decode_row_fields(b"one\\ttwo\t\\N")
                .unwrap()
                .into_iter()
                .map(Cow::into_owned)
                .collect::<Vec<_>>(),
            vec![b"one\ttwo".to_vec(), b"\\N".to_vec()]
        );
        assert!(decode_row_fields(b"bad\\q").is_err());
    }

    #[test]
    fn borrows_unescaped_fields_and_null_markers() {
        let fields = decode_row_fields(b"plain\tone\\ttwo\t\\N").unwrap();
        assert!(matches!(&fields[0], Cow::Borrowed(_)));
        assert!(matches!(&fields[1], Cow::Owned(_)));
        assert!(matches!(&fields[2], Cow::Borrowed(_)));
        assert_eq!(fields[1].as_ref(), b"one\ttwo");
    }

    #[test]
    fn decodes_hex_escapes_with_shared_decoder() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all(b"value\nhello\\x20world\n").unwrap();
        assert_eq!(writer.output, b"Row 1:\n  value: hello world\n\n");
    }

    #[test]
    fn prints_rows_in_column_value_form_even_with_fragmented_writes() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all(b"number\tvalue\n1\t").unwrap();
        writer.flush().unwrap();
        writer.write_all(b"alpha\n2\tbeta\n").unwrap();
        assert_eq!(
            writer.output,
            b"Row 1:\n  number: 1\n  value: alpha\n\nRow 2:\n  number: 2\n  value: beta\n\n"
        );
    }

    #[test]
    fn escapes_terminal_controls_in_headers_and_values() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all(b"na\x1bme\nva\x00lue\x7f\n").unwrap();
        assert_eq!(writer.output, b"Row 1:\n  na\\x1Bme: va\\x00lue\\x7F\n\n");
    }

    #[test]
    fn header_only_results_print_nothing() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all(b"number\n").unwrap();
        writer.flush().unwrap();
        assert!(writer.output.is_empty());
    }

    #[test]
    fn rejects_mismatched_column_counts_before_output() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all(b"a\tb\n").unwrap();
        assert_eq!(
            writer.write_all(b"one\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(writer.output.is_empty());
    }

    #[test]
    fn rejects_rows_larger_than_one_megabyte() {
        let mut writer = VerticalWriter::new(Vec::new());
        assert_eq!(
            writer
                .write_all(&vec![b'a'; MAX_LINE_BYTES + 1])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}

#[cfg(test)]
mod decoded_vertical_regressions {
    use super::*;

    #[test]
    fn decodes_sql_cells_and_keeps_controls_terminal_safe() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer
            .write_all(b"value\none\\ttwo\\nthree\\\\end\n")
            .unwrap();
        assert_eq!(
            writer.output,
            b"Row 1:\n  value: one\\x09two\\x0Athree\\end\n\n"
        );
    }

    #[test]
    fn refuses_bad_escapes_before_printing_a_row() {
        let mut writer = VerticalWriter::new(Vec::new());
        writer.write_all(b"value\n").unwrap();
        assert!(writer.write_all(b"bad\\q\n").is_err());
        assert!(writer.output.is_empty());
    }
}
