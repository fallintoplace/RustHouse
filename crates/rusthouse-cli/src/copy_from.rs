//! Bounded streaming CSV and TSV loaders for COPY FROM local files.
use rusthouse_query::{CopyFormat, CopyFrom, parse_json_each_row_fields};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

const MAX_CSV_RECORD_BYTES: usize = 8 * 1024 * 1024;
const MAX_CSV_DATA_RECORDS: usize = 100_000;
const MAX_CSV_RECORDS: usize = MAX_CSV_DATA_RECORDS + 1;
#[cfg(test)]
const MAX_TSV_RECORDS: usize = MAX_CSV_RECORDS; // legacy in-memory parser tests
const MAX_TSV_RECORD_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Start,
    Unquoted,
    Quoted,
    AfterQuote,
}

fn push_field(
    row: &mut Vec<Option<String>>,
    field: &mut String,
    quoted: bool,
) -> Result<(), String> {
    if row.len() >= 128 {
        return Err("CSV records support at most 128 columns".into());
    }
    let value = std::mem::take(field);
    row.push(if !quoted && (value.is_empty() || value == "\\N") {
        None
    } else {
        Some(value)
    });
    Ok(())
}

fn push_record(
    records: &mut Vec<Vec<Option<String>>>,
    row: &mut Vec<Option<String>>,
) -> Result<(), String> {
    if records.len() >= MAX_CSV_RECORDS {
        return Err("CSV supports at most 100001 records including the header".into());
    }
    records.push(std::mem::take(row));
    Ok(())
}

fn parse_plain_csv(data: &str) -> Result<Vec<Vec<Option<String>>>, String> {
    let mut records = Vec::new();
    let mut width_hint = 0;
    for line in data.split_terminator('\n') {
        let mut row = Vec::with_capacity(width_hint);
        for field in line.split(',') {
            if row.len() >= 128 {
                return Err("CSV records support at most 128 columns".into());
            }
            row.push(if field.is_empty() || field == "\\N" {
                None
            } else {
                Some(field.to_owned())
            });
        }
        width_hint = row.len();
        push_record(&mut records, &mut row)?;
    }
    Ok(records)
}

fn parse_csv(data: &str) -> Result<Vec<Vec<Option<String>>>, String> {
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    // Unquoted CSV needs neither a character state machine nor incremental
    // String::push calls. Reuse the validated bounded record path.
    if !data
        .as_bytes()
        .iter()
        .any(|&byte| byte == b'"' || byte == b'\r')
    {
        return parse_plain_csv(data);
    }
    let mut records = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut state = State::Start;
    let mut chars = data.chars().peekable();
    let mut active = false;
    while let Some(ch) = chars.next() {
        if matches!(state, State::Quoted) {
            if ch == '"' {
                state = State::AfterQuote;
            } else {
                field.push(ch);
            }
            active = true;
            continue;
        }
        if matches!(state, State::AfterQuote) && ch == '"' {
            field.push('"');
            state = State::Quoted;
            continue;
        }
        if matches!(ch, ',' | '\n' | '\r') {
            push_field(&mut row, &mut field, matches!(state, State::AfterQuote))?;
            state = State::Start;
            if ch == ',' {
                active = true;
                continue;
            }
            if ch == '\r' && chars.next() != Some('\n') {
                return Err("CSV requires CRLF for carriage returns".into());
            }
            push_record(&mut records, &mut row)?;
            active = false;
            continue;
        }
        match state {
            State::Start => {
                if ch == '"' {
                    state = State::Quoted;
                } else {
                    field.push(ch);
                    state = State::Unquoted;
                }
            }
            State::Unquoted if ch == '"' => {
                return Err("unexpected quote in unquoted CSV field".into());
            }
            State::Unquoted => field.push(ch),
            State::AfterQuote => {
                return Err("unexpected text after quoted CSV field".into());
            }
            State::Quoted => unreachable!(),
        }
        active = true;
    }
    if state == State::Quoted {
        return Err("unterminated quoted CSV field".into());
    }
    if active {
        push_field(&mut row, &mut field, state == State::AfterQuote)?;
        push_record(&mut records, &mut row)?;
    }
    Ok(records)
}

fn decode_tsv_cell(field: &str) -> Result<String, String> {
    if !field.as_bytes().contains(&b'\\') {
        return Ok(field.to_owned());
    }
    // Decode plain UTF-8 spans in bulk rather than encoding each character
    // again. The shared decoder rejects unsupported or truncated escapes.
    let decoded =
        crate::tsv_escape::decode_tsv_field(field.as_bytes()).map_err(|error| error.to_string())?;
    String::from_utf8(decoded).map_err(|_| "invalid UTF-8 in TSV field".into())
}

fn parse_delimited_record(line: &str, delimiter: char) -> Result<Vec<Option<String>>, String> {
    if line.contains('\r') {
        return Err("TSV requires CRLF for carriage returns".into());
    }
    let mut row = Vec::new();
    for field in line.split(delimiter) {
        if row.len() >= 128 {
            return Err("TSV records support at most 128 columns".into());
        }
        if field == "\\N" {
            row.push(None);
        } else {
            row.push(Some(decode_tsv_cell(field)?));
        }
    }
    Ok(row)
}

#[cfg(test)]
fn parse_tsv_record(line: &str) -> Result<Vec<Option<String>>, String> {
    parse_delimited_record(line, '\t')
}

#[cfg(test)]
fn parse_tsv(data: &str) -> Result<Vec<Vec<Option<String>>>, String> {
    let input = data.strip_prefix('\u{feff}').unwrap_or(data);
    if input.ends_with('\r') {
        return Err("TSV requires CRLF for carriage returns".into());
    }
    let mut records = Vec::new();
    for line in input.split_terminator('\n') {
        if records.len() >= MAX_TSV_RECORDS {
            return Err("TSV supports at most 100001 records including the header".into());
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        records.push(parse_tsv_record(line)?);
    }
    Ok(records)
}

struct TsvRecords {
    reader: BufReader<Box<dyn Read>>,
    buffer: Vec<u8>,
    first: bool,
    delimiter: char,
}

impl TsvRecords {
    fn next_record(&mut self) -> Result<Option<Vec<Option<String>>>, String> {
        self.buffer.clear();
        // Bound a single record even when the file itself is many gigabytes.
        let size = self
            .reader
            .by_ref()
            .take((MAX_TSV_RECORD_BYTES + 1) as u64)
            .read_until(b'\n', &mut self.buffer)
            .map_err(|error| format!("cannot read COPY TSV record: {error}"))?;
        if size == 0 {
            return Ok(None);
        }
        if size > MAX_TSV_RECORD_BYTES {
            return Err("COPY TSV record exceeds 8 MiB".into());
        }
        let line = std::str::from_utf8(&self.buffer).map_err(|_| "COPY requires UTF-8 data")?;
        let line = if self.first {
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        self.first = false;
        let line = if let Some(line) = line.strip_suffix('\n') {
            line.strip_suffix('\r').unwrap_or(line)
        } else {
            line
        };
        Ok(Some(parse_delimited_record(line, self.delimiter)?))
    }
}

fn run_tsv(database: &Path, statement: &CopyFrom) -> Result<(), String> {
    let reader: Box<dyn Read> = if statement.path == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(
            File::open(&statement.path)
                .map_err(|error| format!("cannot open COPY file {}: {error}", statement.path))?,
        )
    };
    let mut input = TsvRecords {
        reader: BufReader::new(reader),
        buffer: Vec::new(),
        first: true,
        delimiter: if statement.format == CopyFormat::Ssv {
            ';'
        } else {
            '\t'
        },
    };
    let mut columns = statement.columns.clone();
    if statement.header {
        let headings = input
            .next_record()?
            .ok_or("COPY file is missing its header")?;
        let names = headings
            .into_iter()
            .map(|value| value.ok_or("COPY header cannot contain empty columns"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut unique = HashSet::new();
        if names.iter().any(|name| !unique.insert(name.clone())) {
            return Err("duplicate COPY header column".into());
        }
        if let Some(expected) = columns.as_ref() {
            if expected != &names {
                return Err("COPY header does not match explicit column list".into());
            }
        } else {
            columns = Some(names);
        }
    }
    crate::database::insert_stream(database, &statement.table, columns, || input.next_record())
}

/// A quoted CSV record may span many physical lines. Buffer only that record,
/// never the full input, and let the existing parser preserve quoted NULL rules.
struct CsvRecords {
    reader: BufReader<Box<dyn Read>>,
    buffer: Vec<u8>,
    first: bool,
}

impl CsvRecords {
    fn next_record(&mut self) -> Result<Option<Vec<Option<String>>>, String> {
        self.buffer.clear();
        let mut quoted = false;
        loop {
            let remaining = MAX_CSV_RECORD_BYTES
                .checked_sub(self.buffer.len())
                .ok_or("COPY CSV record exceeds 8 MiB")?;
            if remaining == 0 {
                return Err("COPY CSV record exceeds 8 MiB".into());
            }
            // Append directly into the reusable record buffer. COPY can
            // consume billions of short CSV records, so allocating a fresh
            // line Vec for every single row is expensive.
            let previous_len = self.buffer.len();
            let read = self
                .reader
                .by_ref()
                .take((remaining + 1) as u64)
                .read_until(b'\n', &mut self.buffer)
                .map_err(|error| format!("cannot read COPY CSV record: {error}"))?;
            if read == 0 {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                break;
            }
            let added = &self.buffer[previous_len..];
            let finished_line = added.ends_with(b"\n");
            for &byte in added {
                if byte == b'"' {
                    quoted = !quoted;
                }
            }
            if self.buffer.len() > MAX_CSV_RECORD_BYTES {
                return Err("COPY CSV record exceeds 8 MiB".into());
            }
            if finished_line && !quoted {
                break;
            }
        }
        let text = std::str::from_utf8(&self.buffer).map_err(|_| "COPY requires UTF-8 data")?;
        let text = if self.first {
            text.strip_prefix('\u{feff}').unwrap_or(text)
        } else {
            text
        };
        self.first = false;
        // The parser may strip a BOM at the beginning of a record. Only the
        // first physical CSV record may have a BOM; reject later BOMs instead.
        if text.starts_with('\u{feff}') {
            return Err("COPY CSV BOM is only allowed at the beginning".into());
        }
        let mut rows = parse_csv(text)?;
        if rows.len() != 1 {
            return Err("COPY CSV record is empty or malformed".into());
        }
        Ok(Some(rows.remove(0)))
    }
}

fn run_csv(database: &Path, statement: &CopyFrom) -> Result<(), String> {
    let reader: Box<dyn Read> = if statement.path == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(
            File::open(&statement.path)
                .map_err(|error| format!("cannot open COPY file {}: {error}", statement.path))?,
        )
    };
    let mut input = CsvRecords {
        reader: BufReader::new(reader),
        buffer: Vec::new(),
        first: true,
    };
    let mut columns = statement.columns.clone();
    if statement.header {
        let headings = input
            .next_record()?
            .ok_or("COPY file is missing its header")?;
        let names = headings
            .into_iter()
            .map(|value| value.ok_or("COPY header cannot contain empty columns"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut unique = HashSet::new();
        if names.iter().any(|name| !unique.insert(name.clone())) {
            return Err("duplicate COPY header column".into());
        }
        if let Some(expected) = columns.as_ref() {
            if expected != &names {
                return Err("COPY header does not match explicit column list".into());
            }
        } else {
            columns = Some(names);
        }
    }
    crate::database::insert_stream(database, &statement.table, columns, || input.next_record())
}

/// Read ClickHouse JSONEachRow streams a line at a time. The strict JSON
/// parser bounds nesting and field sizes, while this reader bounds line length.
const MAX_JSONEACHROW_RECORD_BYTES: usize = 1024 * 1024;

struct JsonEachRowRecords {
    reader: BufReader<Box<dyn Read>>,
    buffer: Vec<u8>,
    first: bool,
    column_indices: HashMap<String, usize>,
    column_count: usize,
}

impl JsonEachRowRecords {
    fn next_record(&mut self) -> Result<Option<Vec<Option<String>>>, String> {
        loop {
            self.buffer.clear();
            let read = self.reader.by_ref()
                .take((MAX_JSONEACHROW_RECORD_BYTES + 1) as u64)
                .read_until(b'\n', &mut self.buffer)
                .map_err(|error| format!("cannot read JSONEachRow record: {error}"))?;
            if read == 0 {
                return Ok(None);
            }
            if self.buffer.len() > MAX_JSONEACHROW_RECORD_BYTES {
                return Err("JSONEachRow record exceeds 1 MiB".into());
            }
            let text = std::str::from_utf8(&self.buffer)
                .map_err(|_| "JSONEachRow input must be UTF-8")?
                .trim_end_matches(|ch| ch == '\n' || ch == '\r');
            let text = if self.first {
                text.strip_prefix('\u{feff}').unwrap_or(text)
            } else {
                text
            };
            self.first = false;
            if text.starts_with('\u{feff}') {
                return Err("JSONEachRow BOM is only allowed at the beginning".into());
            }
            if text.trim().is_empty() {
                continue;
            }
            let fields = parse_json_each_row_fields(text)?;
            let mut values = vec![None; self.column_count];
            for (name, value) in fields {
                let index = *self.column_indices.get(&name)
                    .ok_or_else(|| format!("unknown JSONEachRow column: {name}"))?;
                values[index] = value;
            }
            return Ok(Some(values));
        }
    }
}

fn run_json_each_row(database: &Path, statement: &CopyFrom) -> Result<(), String> {
    let columns = if let Some(explicit) = statement.columns.clone() {
        explicit
    } else {
        crate::catalog::read_schema(database, &statement.table)?
            .into_iter().map(|column| column.name).collect()
    };
    let column_count = columns.len();
    let mut column_indices = HashMap::with_capacity(column_count);
    for (index, name) in columns.iter().enumerate() {
        if column_indices.insert(name.clone(), index).is_some() {
            return Err(format!("duplicate JSONEachRow column: {name}"));
        }
    }
    let source: Box<dyn Read> = if statement.path == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(File::open(&statement.path)
            .map_err(|error| format!("cannot open COPY file {}: {error}", statement.path))?)
    };
    let mut reader = JsonEachRowRecords {
        reader: BufReader::new(source), buffer: Vec::new(),
        first: true, column_indices, column_count,
    };
    crate::database::insert_stream(
        database, &statement.table, Some(columns), || reader.next_record(),
    )
}

/// Stream a native ClickHouse INSERT ... FORMAT JSONEachRow from stdin.
/// This shares the atomic snapshot writer and schema-aware row mapping used
/// by JSONEachRow COPY without reading the entire body into memory.
pub(super) fn run_insert_json_each_row(
    database: &Path,
    header: &rusthouse_query::InsertFormatHeader,
) -> Result<(), String> {
    run_json_each_row(database, &CopyFrom {
        table: header.table.clone(),
        columns: header.columns.clone(),
        path: "-".into(),
        header: false,
        format: CopyFormat::JsonEachRow,
    })
}

pub(super) fn run(database: &Path, statement: &CopyFrom) -> Result<(), String> {
    match statement.format {
        CopyFormat::Tsv | CopyFormat::Ssv => run_tsv(database, statement),
        CopyFormat::Csv => run_csv(database, statement),
        CopyFormat::JsonEachRow => run_json_each_row(database, statement),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_105_column_records_and_rejects_129() {
        let tsv = vec!["123"; 105].join("\t");
        assert_eq!(parse_tsv_record(&tsv).unwrap().len(), 105);
        let csv = vec!["123"; 105].join(",");
        assert_eq!(parse_csv(&csv).unwrap()[0].len(), 105);
        assert!(parse_tsv_record(&vec!["1"; 129].join("\t")).is_err());
        assert!(parse_csv(&vec!["1"; 129].join(",")).is_err());
    }

    #[test]
    fn csv_parser_accepts_more_than_50000_data_rows_and_still_bounds_records() {
        let input = "1\n".repeat(100_001);
        assert_eq!(parse_csv(&input).unwrap().len(), 100_001);
        let overflow = format!("{input}1\n");
        assert!(parse_csv(&overflow).is_err());
    }

    #[test]
    fn csv_parser_distinguishes_null_and_quoted_empty() {
        let records =
            parse_csv("id,name,note\r\n1,\"Ada, Lovelace\",\r\n2,\"a\"\"b\"\"\",\"\"\r\n").unwrap();
        assert_eq!(records[0][0], Some("id".into()));
        assert_eq!(records[1][1], Some("Ada, Lovelace".into()));
        assert_eq!(records[1][2], None);
        assert_eq!(records[2][1], Some("a\"b\"".into()));
        assert_eq!(records[2][2], Some("".into()));
        assert_eq!(
            parse_csv("\"hello\nworld\",42\n").unwrap()[0][0],
            Some("hello\nworld".into())
        );
    }

    #[test]
    fn plain_csv_fast_path_keeps_nulls_empty_lines_and_limits() {
        let rows = parse_csv("id,value\n1,Ada\n2,\\N\n3,\n\n").unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[1], vec![Some("1".into()), Some("Ada".into())]);
        assert_eq!(rows[2], vec![Some("2".into()), None]);
        assert_eq!(rows[3], vec![Some("3".into()), None]);
        assert_eq!(rows[4], vec![None]);
        assert!(parse_csv(&"x,".repeat(129)).is_err());
    }

    #[test]
    fn csv_import_distinguishes_null_sentinel_and_quoted_text() {
        let rows = parse_csv("id,value\n1,\\N\n2,\"\\N\"\n3,\n4,\"\"\n").unwrap();
        assert_eq!(rows[1], vec![Some("1".into()), None]);
        assert_eq!(rows[2], vec![Some("2".into()), Some("\\N".into())]);
        assert_eq!(rows[3], vec![Some("3".into()), None]);
        assert_eq!(rows[4], vec![Some("4".into()), Some("".into())]);
    }

    #[test]
    fn tsv_decodes_nulls_and_escaped_fields() {
        assert_eq!(
            parse_tsv("id\tname\tnote\r\n1\tAda\\tLovelace\t\\N\r\n2\tGrace\\nHopper\t\n").unwrap(),
            vec![
                vec![Some("id".into()), Some("name".into()), Some("note".into())],
                vec![Some("1".into()), Some("Ada\tLovelace".into()), None],
                vec![
                    Some("2".into()),
                    Some("Grace\nHopper".into()),
                    Some("".into())
                ],
            ]
        );
    }

    #[test]
    fn decodes_tsv_cells_without_row_state() {
        assert_eq!(decode_tsv_cell("plain").unwrap(), "plain");
        assert_eq!(decode_tsv_cell("café-東京").unwrap(), "café-東京");
        assert_eq!(
            decode_tsv_cell("café-東京\\tMünchen").unwrap(),
            "café-東京\tMünchen"
        );
        assert_eq!(decode_tsv_cell("").unwrap(), "");
        assert_eq!(decode_tsv_cell("caf\\xC3\\xA9\\t").unwrap(), "café\t");
        assert!(decode_tsv_cell("\\N").is_err());
    }

    #[test]
    fn imports_hex_escaped_tsv_bytes_and_validates_utf8() {
        let rows = parse_tsv("value\ncaf\\xC3\\xA9\\x01\n").unwrap();
        assert_eq!(rows[1][0], Some("café\u{1}".into()));
        for invalid in ["\\xG1\n", "\\x1\n", "\\xFF\n"] {
            assert!(parse_tsv(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn invalid_tsv_escapes_are_rejected() {
        for input in ["bad\\q\n", "bad\\\n", "a\\Nvalue\n", "a\rb\n", "a\r"] {
            assert!(parse_tsv(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn invalid_csv_fails_before_insert() {
        for input in ["\"unterminated", "a\"bad,b", "\"a\"x,b", "bad\rline"] {
            assert!(parse_csv(input).is_err(), "{input}");
        }
    }
}

#[cfg(test)]
mod utf8_bom_regressions {
    use super::*;

    #[test]
    fn ignores_file_bom_without_removing_embedded_unicode_markers() {
        assert_eq!(
            parse_csv("\u{feff}id,name\r\n1,Ada\r\n").unwrap(),
            parse_csv("id,name\r\n1,Ada\r\n").unwrap()
        );
        assert_eq!(
            parse_csv("id,\u{feff}name\n").unwrap()[0][1],
            Some("\u{feff}name".to_owned())
        );
    }
}

#[cfg(test)]
mod copy_file_capacity_tests {
    use super::*;

    #[test]
    fn csv_streams_quoted_newlines_and_preserves_null_semantics() {
        let input = b"id,value\n1,\"hello\nworld\"\n2,\n3,\"\"\n";
        let mut reader = CsvRecords {
            reader: BufReader::new(Box::new(std::io::Cursor::new(input.to_vec()))),
            buffer: Vec::new(),
            first: true,
        };
        assert_eq!(
            reader.next_record().unwrap().unwrap(),
            vec![Some("id".into()), Some("value".into())]
        );
        assert_eq!(
            reader.next_record().unwrap().unwrap(),
            vec![Some("1".into()), Some("hello\nworld".into())]
        );
        assert_eq!(
            reader.next_record().unwrap().unwrap(),
            vec![Some("2".into()), None]
        );
        assert_eq!(
            reader.next_record().unwrap().unwrap(),
            vec![Some("3".into()), Some("".into())]
        );
        assert!(reader.next_record().unwrap().is_none());
    }

    #[test]
    fn csv_rejects_truncated_record_and_per_record_oversize() {
        let mut bad = CsvRecords {
            reader: BufReader::new(Box::new(std::io::Cursor::new(b"\"unterminated\n".to_vec()))),
            buffer: Vec::new(),
            first: true,
        };
        assert!(bad.next_record().is_err());

        let oversized = vec![b'a'; MAX_CSV_RECORD_BYTES + 1];
        let mut giant = CsvRecords {
            reader: BufReader::new(Box::new(std::io::Cursor::new(oversized))),
            buffer: Vec::new(),
            first: true,
        };
        assert!(giant.next_record().unwrap_err().contains("8 MiB"));
    }
}

#[cfg(test)]
mod csv_large_ingest_allocation_tests {
    use super::*;

    #[test]
    fn csv_reuses_record_allocation_across_large_stream() {
        let input = b"42,region-01,10923\n".repeat(20_000);
        let mut reader = CsvRecords {
            reader: BufReader::new(Box::new(std::io::Cursor::new(input))),
            buffer: Vec::new(),
            first: true,
        };
        let first = reader.next_record().unwrap().unwrap();
        assert_eq!(first[0], Some("42".to_string()));
        let pointer = reader.buffer.as_ptr();
        let capacity = reader.buffer.capacity();
        for _ in 1..20_000 {
            let row = reader.next_record().unwrap().unwrap();
            assert_eq!(row[1].as_deref(), Some("region-01"));
            assert_eq!(reader.buffer.capacity(), capacity);
            assert_eq!(reader.buffer.as_ptr(), pointer);
        }
        assert!(reader.next_record().unwrap().is_none());
    }

    #[test]
    fn shared_buffer_preserves_multiline_quote_boundaries() {
        let input = b"1,\"alpha\nbeta\",9\n2,\"a\"\"b\",10\n";
        let mut reader = CsvRecords {
            reader: BufReader::new(Box::new(std::io::Cursor::new(input.to_vec()))),
            buffer: Vec::new(),
            first: true,
        };
        assert_eq!(reader.next_record().unwrap().unwrap(),
            vec![Some("1".into()), Some("alpha\nbeta".into()), Some("9".into())]);
        assert_eq!(reader.next_record().unwrap().unwrap(),
            vec![Some("2".into()), Some("a\"b".into()), Some("10".into())]);
    }
}
