//! Convert bounded SQL TSV results into RFC 4180-style CSV on the HTTP interface.
//! Decode TSV escapes before quoting values. NULL is represented as unquoted \\N.
use super::jsonl::decode_tsv_value;
use rusthouse_format::write_quoted_csv_field as write_quoted_field;
use std::io::{self, Write};

const MAX_LINE_BYTES: usize = 1024 * 1024;

pub(super) struct CsvWriter<W: Write> {
    output: W,
    line: Vec<u8>,
    encoded: Vec<u8>,
    columns: Option<usize>,
}

impl<W: Write> CsvWriter<W> {
    pub(super) fn new(output: W) -> Self {
        Self {
            output,
            line: Vec::new(),
            encoded: Vec::new(),
            columns: None,
        }
    }

    fn finish_row(&mut self) -> io::Result<()> {
        // Encode into a temporary buffer before publishing anything, so
        // malformed rows never leak partially written CSV to the client.
        self.encoded.clear();
        self.encoded.reserve(self.line.len().saturating_add(4));
        let mut field_count = 0usize;
        for field in self.line.split(|byte| *byte == b'\t') {
            if field_count != 0 {
                self.encoded.write_all(b",")?;
            }
            write_csv_field(&mut self.encoded, field)?;
            field_count += 1;
        }
        if self.columns.is_some_and(|columns| columns != field_count) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CSV row has a different column count",
            ));
        }
        self.encoded.write_all(b"\r\n")?;
        self.output.write_all(&self.encoded)?;
        self.columns = Some(field_count);
        self.line.clear();
        Ok(())
    }
}

fn write_csv_field<W: Write>(output: &mut W, field: &[u8]) -> io::Result<()> {
    if field == b"\\N" {
        return output.write_all(b"\\N");
    }
    let decoded = if field.contains(&b'\\') {
        Some(decode_tsv_value(field)?)
    } else {
        None
    };
    let value = match decoded.as_deref() {
        Some(value) => value,
        None => std::str::from_utf8(field)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8 field"))?,
    };
    let needs_quotes = value == "\\N"
        || value
            .bytes()
            .any(|byte| matches!(byte, b',' | b'"' | b'\t' | b'\n' | b'\r'));
    if !needs_quotes {
        return output.write_all(value.as_bytes());
    }
    write_quoted_field(output, value.as_bytes())
}

impl<W: Write> Write for CsvWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for chunk in buf.split_inclusive(|&byte| byte == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            let data = if complete {
                &chunk[..chunk.len() - 1]
            } else {
                chunk
            };
            if data.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "CSV row exceeds 1 MiB",
                ));
            }
            self.line.extend_from_slice(data);
            if complete {
                self.finish_row()?;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

#[cfg(test)]
#[path = "csv/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "csv/literal_null_marker_regressions.rs"]
mod literal_null_marker_regressions;

#[cfg(test)]
#[path = "csv/single_pass_row_tests.rs"]
mod single_pass_row_tests;
