//! Bounded CSV loader and streaming TSV loader for COPY FROM local files.
use rusthouse_query::{CopyFormat, CopyFrom, Insert};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

const MAX_COPY_BYTES: usize = 8 * 1024 * 1024;
const MAX_CSV_DATA_RECORDS: usize = 50_000;
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
        return Err("CSV supports at most 50001 records including the header".into());
    }
    records.push(std::mem::take(row));
    Ok(())
}

fn parse_csv(data: &str) -> Result<Vec<Vec<Option<String>>>, String> {
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
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
    let mut decoded = Vec::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            let mut bytes = [0; 4];
            decoded.extend_from_slice(ch.encode_utf8(&mut bytes).as_bytes());
            continue;
        }
        let escaped = chars.next().ok_or("TSV ends with an incomplete escape")?;
        match escaped {
            't' => decoded.push(b'\t'),
            'n' => decoded.push(b'\n'),
            'r' => decoded.push(b'\r'),
            '0' => decoded.push(0),
            'b' => decoded.push(8),
            'f' => decoded.push(12),
            'v' => decoded.push(11),
            '\\' => decoded.push(b'\\'),
            'x' => {
                let high = chars
                    .next()
                    .and_then(|digit| digit.to_digit(16))
                    .ok_or("invalid TSV hex escape")?;
                let low = chars
                    .next()
                    .and_then(|digit| digit.to_digit(16))
                    .ok_or("invalid TSV hex escape")?;
                decoded.push(((high << 4) | low) as u8);
            }
            'N' => return Err("TSV NULL marker must occupy the whole field".into()),
            _ => return Err(format!("unsupported TSV escape: \\{escaped}")),
        }
    }
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
            return Err("TSV supports at most 50001 records including the header".into());
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

pub(super) fn run(database: &Path, statement: &CopyFrom) -> Result<(), String> {
    if matches!(statement.format, CopyFormat::Tsv | CopyFormat::Ssv) {
        return run_tsv(database, statement);
    }
    if statement.path == "-" {
        return Err("COPY FROM '-' requires TSV format".into());
    }
    let mut data = Vec::new();
    File::open(&statement.path)
        .map_err(|error| format!("cannot open COPY file {}: {error}", statement.path))?
        .take((MAX_COPY_BYTES + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|error| format!("cannot read COPY file {}: {error}", statement.path))?;
    if data.len() > MAX_COPY_BYTES {
        return Err("COPY file exceeds 8 MiB".into());
    }
    let input = std::str::from_utf8(&data).map_err(|_| "COPY requires UTF-8 data")?;
    let mut records = parse_csv(input)?;
    let mut columns = statement.columns.clone();
    if statement.header {
        if records.is_empty() {
            return Err("COPY file is missing its header".into());
        }
        let headings = records.remove(0);
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
    let max_data_records = MAX_CSV_DATA_RECORDS;
    if records.len() > max_data_records {
        return Err(format!(
            "COPY supports at most {max_data_records} data rows"
        ));
    }
    if records.is_empty() {
        // Empty imports should not create needless immutable parts.
        crate::database::select_source(database, &statement.table)?;
        println!("Inserted 0 rows into {}", statement.table);
        return Ok(());
    }
    let insert = Insert {
        table: statement.table.clone(),
        columns,
        rows: records,
    };
    if insert.rows.len() > 4096 {
        crate::database::insert_many(database, &insert)
    } else {
        crate::database::insert(database, &insert)
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
