//! INSERT ... SELECT streams via an on-disk spool into one atomic part snapshot.
//! The source query finishes before publishing, keeping self-INSERT consistent.
use rusthouse_query::{InsertSelect, Source, execute};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_ROW_BYTES: usize = 8 * 1024 * 1024;
static NEXT_SPOOL: AtomicUsize = AtomicUsize::new(0);

struct Spool(PathBuf);

impl Drop for Spool {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn new_spool(database: &Path) -> Result<(Spool, File), String> {
    let path = database.join(format!(
        ".rusthouse-insert-select-{}-{}.tmp",
        std::process::id(),
        NEXT_SPOOL.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("cannot create INSERT SELECT spool: {error}"))?;
    Ok((Spool(path), file))
}

fn decode_field(value: &str) -> Result<Option<String>, String> {
    if value == r"\N" {
        return Ok(None);
    }
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let escaped = bytes
            .next()
            .ok_or("invalid trailing escape in SELECT result")?;
        match escaped {
            b'\\' => decoded.push(b'\\'),
            b't' => decoded.push(b'\t'),
            b'n' => decoded.push(b'\n'),
            b'r' => decoded.push(b'\r'),
            b'0' => decoded.push(0),
            b'x' => {
                let hi = bytes
                    .next()
                    .ok_or("incomplete hex escape in SELECT result")?;
                let lo = bytes
                    .next()
                    .ok_or("incomplete hex escape in SELECT result")?;
                let digit = |byte: u8| match byte {
                    b'0'..=b'9' => Some(byte - b'0'),
                    b'a'..=b'f' => Some(byte - b'a' + 10),
                    b'A'..=b'F' => Some(byte - b'A' + 10),
                    _ => None,
                };
                let hi = digit(hi).ok_or("invalid hex escape in SELECT result")?;
                let lo = digit(lo).ok_or("invalid hex escape in SELECT result")?;
                decoded.push((hi << 4) | lo);
            }
            _ => return Err("invalid escape in SELECT result".into()),
        }
    }
    String::from_utf8(decoded)
        .map(Some)
        .map_err(|_| "invalid UTF-8 in SELECT result".into())
}

/// Read newline-terminated records without allocating for a malicious or
/// accidentally enormous row. The copy publisher still validates all types.
fn read_record<R: BufRead>(reader: &mut R, record: &mut Vec<u8>) -> Result<bool, String> {
    record.clear();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| format!("cannot read INSERT SELECT spool: {error}"))?;
        if available.is_empty() {
            if record.is_empty() {
                return Ok(false);
            }
            return Err("incomplete SELECT result record".into());
        }
        let length = available
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if record.len().saturating_add(length) > MAX_ROW_BYTES {
            return Err("INSERT SELECT row exceeds 8 MiB".into());
        }
        record.extend_from_slice(&available[..length]);
        reader.consume(length);
        if record.last() == Some(&b'\n') {
            return Ok(true);
        }
    }
}

fn decode_row(record: &[u8]) -> Result<Vec<Option<String>>, String> {
    let record = record
        .strip_suffix(b"\n")
        .ok_or("incomplete SELECT result record")?;
    let record = std::str::from_utf8(record).map_err(|_| "invalid UTF-8 in SELECT result")?;
    if record.split('\t').count() > 128 {
        return Err("INSERT SELECT supports at most 128 columns".into());
    }
    record
        .split('\t')
        .map(decode_field)
        .collect::<Result<Vec<_>, _>>()
}

pub(super) fn run(database: &Path, statement: &InsertSelect) -> Result<(), String> {
    // Hold one write lock throughout source resolution, full source execution,
    // target validation, and the final immutable snapshot publication.
    crate::database::with_write_lock(database, || {
        let mut query = statement.select.clone();
        let part = if let Some(Source::Table(table)) = query.source.as_ref() {
            let path = crate::database::select_source(database, table)?;
            query.source = Some(Source::Table("part".into()));
            Some(path)
        } else {
            None
        };
        let (spool, file) = new_spool(database)?;
        {
            let mut writer = BufWriter::new(file);
            execute(&query, part.as_deref(), &mut writer).map_err(|error| error.to_string())?;
            writer.flush().map_err(|error| format!("cannot flush INSERT SELECT spool: {error}"))?;
        }
        let file = File::open(&spool.0)
            .map_err(|error| format!("cannot reopen INSERT SELECT spool: {error}"))?;
        let mut reader = BufReader::new(file);
        let mut record = Vec::with_capacity(64 * 1024);
        if !read_record(&mut reader, &mut record)? || record == b"\n" {
            return Err("missing SELECT result header".into());
        }
        std::str::from_utf8(&record).map_err(|_| "invalid SELECT result header")?;
        crate::database::insert_stream_locked(
            database,
            &statement.table,
            statement.columns.clone(),
            || {
                if !read_record(&mut reader, &mut record)? {
                    return Ok(None);
                }
                decode_row(&record).map(Some)
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_records_preserve_nulls_escapes_and_line_boundaries() {
        let content = b"id\ttext\tnote\n1\tline\\nnext\t\\N\n2\t\\\\N\tback\\\\slash\n";
        let mut input = BufReader::with_capacity(4, &content[..]);
        let mut record = Vec::new();
        assert!(read_record(&mut input, &mut record).unwrap());
        assert_eq!(&record, b"id\ttext\tnote\n");
        assert!(read_record(&mut input, &mut record).unwrap());
        let first = decode_row(&record).unwrap();
        assert_eq!(first[1], Some("line\nnext".into()));
        assert_eq!(first[2], None);
        assert!(read_record(&mut input, &mut record).unwrap());
        let second = decode_row(&record).unwrap();
        assert_eq!(second[1], Some(r"\N".into()));
        assert_eq!(second[2], Some(r"back\slash".into()));
        assert!(!read_record(&mut input, &mut record).unwrap());
    }

    #[test]
    fn never_accepts_truncated_or_oversized_rows() {
        let mut record = Vec::new();
        assert!(read_record(&mut &b"unterminated"[..], &mut record).is_err());
        assert!(decode_row(b"field").is_err());
        let too_large = vec![b'x'; MAX_ROW_BYTES + 1];
        assert!(read_record(&mut too_large.as_slice(), &mut record).is_err());
        assert!(decode_row(b"invalid\\xZZ\n").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn locks_before_resolving_a_source_table() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let database = std::env::temp_dir().join(format!(
            "rusthouse-insert-select-lock-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let statement =
            rusthouse_query::parse_insert_select("INSERT INTO target SELECT id FROM missing")
                .unwrap()
                .unwrap();
        crate::database::with_write_lock(&database, || {
            let error = run(&database, &statement).unwrap_err();
            assert!(error.contains("cannot acquire database write lock"), "{error}");
            Ok(())
        })
        .unwrap();
        fs::remove_dir_all(database).unwrap();
    }
}
