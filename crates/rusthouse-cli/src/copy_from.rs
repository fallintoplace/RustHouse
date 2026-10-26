//! Strict, bounded CSV loader for COPY table FROM 'local-file'.
use rusthouse_query::{CopyFrom, Insert};
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const MAX_CSV_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;

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
    if row.len() >= 64 {
        return Err("CSV records support at most 64 columns".into());
    }
    let value = std::mem::take(field);
    row.push(if !quoted && value.is_empty() {
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
    if records.len() > MAX_RECORDS {
        return Err("CSV supports at most 4096 records".into());
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

pub(super) fn run(database: &Path, statement: &CopyFrom) -> Result<(), String> {
    let mut data = Vec::new();
    File::open(&statement.path)
        .map_err(|error| format!("cannot open COPY file {}: {error}", statement.path))?
        .take((MAX_CSV_BYTES + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|error| format!("cannot read COPY file {}: {error}", statement.path))?;
    if data.len() > MAX_CSV_BYTES {
        return Err("COPY file exceeds 8 MiB".into());
    }
    let input = std::str::from_utf8(&data).map_err(|_| "COPY requires UTF-8 CSV")?;
    let mut records = parse_csv(input)?;
    let mut columns = statement.columns.clone();
    if statement.header {
        if records.is_empty() {
            return Err("COPY file is missing its CSV header".into());
        }
        let headings = records.remove(0);
        let names = headings
            .into_iter()
            .map(|value| value.ok_or("CSV header cannot contain empty columns"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut unique = HashSet::new();
        if names.iter().any(|name| !unique.insert(name.clone())) {
            return Err("duplicate CSV header column".into());
        }
        if let Some(expected) = columns.as_ref() {
            if expected != &names {
                return Err("CSV header does not match explicit COPY column list".into());
            }
        } else {
            columns = Some(names);
        }
    }
    if records.len() > MAX_RECORDS {
        return Err("COPY supports at most 4096 data rows".into());
    }
    if records.is_empty() {
        // Empty imports should not create needless immutable parts.
        crate::database::select_source(database, &statement.table)?;
        println!("Inserted 0 rows into {}", statement.table);
        return Ok(());
    }
    crate::database::insert(
        database,
        &Insert {
            table: statement.table.clone(),
            columns,
            rows: records,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
