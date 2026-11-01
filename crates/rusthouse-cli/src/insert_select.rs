//! INSERT ... SELECT streams via an on-disk spool into one atomic part snapshot.
//! The source query finishes before publishing, keeping self-INSERT consistent.
use rusthouse_query::{InsertSelect, Source, execute};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
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

fn open_insert_spool_candidates(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<(Spool, File), String> {
    for path in paths {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((Spool(path), file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot create INSERT SELECT spool: {error}")),
        }
    }
    Err("cannot create INSERT SELECT spool: no unique temporary filename".into())
}

fn create_insert_spool(database: &Path) -> Result<(Spool, File), String> {
    // Stale files from a crashed process or reused PID must not block
    // subsequent INSERT SELECT queries. Never overwrite an existing spool.
    open_insert_spool_candidates((0..64).map(|_| {
        database.join(format!(
            ".rusthouse-insert-select-{}-{}.tmp",
            std::process::id(),
            NEXT_SPOOL.fetch_add(1, Ordering::Relaxed)
        ))
    }))
}

fn decode_field(value: &str) -> Result<Option<String>, String> {
    if value == r"\N" {
        return Ok(None);
    }
    // SELECT rows use the same TSV escape language as COPY and CLI output.
    // Reuse its checked decoder so all control and hex escapes round-trip.
    let decoded = crate::tsv_escape::decode_tsv_field(value.as_bytes())
        .map_err(|error| format!("invalid escape in SELECT result: {error}"))?;
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
    if record.split('\t').nth(128).is_some() {
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
        let (spool, file) = create_insert_spool(database)?;
        {
            let mut writer = BufWriter::new(file);
            execute(&query, part.as_deref(), &mut writer).map_err(|error| error.to_string())?;
            writer
                .flush()
                .map_err(|error| format!("cannot flush INSERT SELECT spool: {error}"))?;
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
    fn retries_occupied_spool_names_without_replacing_existing_files() {
        let directory = std::env::temp_dir().join(format!(
            "rusthouse-spool-collision-{}-{}",
            std::process::id(),
            NEXT_SPOOL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let occupied = directory.join("occupied.tmp");
        let available = directory.join("available.tmp");
        fs::write(&occupied, b"keep-original").unwrap();
        let (spool, mut file) =
            open_insert_spool_candidates([occupied.clone(), available.clone()]).unwrap();
        file.write_all(b"new").unwrap();
        drop(file);
        assert_eq!(fs::read(&available).unwrap(), b"new");
        drop(spool);
        assert_eq!(fs::read(&occupied).unwrap(), b"keep-original");
        assert!(!available.exists());
        fs::remove_dir_all(directory).unwrap();
    }

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
            assert!(
                error.contains("cannot acquire database write lock"),
                "{error}"
            );
            Ok(())
        })
        .unwrap();
        fs::remove_dir_all(database).unwrap();
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;

    #[test]
    fn spool_rows_accept_128_columns_but_reject_129() {
        let within_limit = format!("{}\n", vec!["x"; 128].join("\t"));
        assert_eq!(decode_row(within_limit.as_bytes()).unwrap().len(), 128);
        let over_limit = format!("{}\n", vec!["x"; 129].join("\t"));
        assert!(decode_row(over_limit.as_bytes()).is_err());
    }
}

#[cfg(test)]
mod shared_insert_select_decoder_tests {
    use super::*;

    #[test]
    fn round_trips_all_tsv_control_escapes() {
        assert_eq!(
            decode_field(r"\b\f\v\t\n\r\0").unwrap(),
            Some("\u{0008}\u{000c}\u{000b}\t\n\r\0".into())
        );
        assert_eq!(decode_field(r"\x41\x42\\").unwrap(), Some("AB\\".into()));
        assert_eq!(decode_field(r"\N").unwrap(), None);
        assert_eq!(decode_field(r"\\N").unwrap(), Some(r"\N".into()));
    }

    #[test]
    fn malformed_sequences_still_fail_before_insert() {
        for input in [r"\x", r"\xZ0", r"\q", "\\", r"\xff"] {
            assert!(decode_field(input).is_err(), "{input}");
        }
    }

    #[test]
    fn long_plain_fields_keep_unicode_and_exact_contents() {
        let value = "café🚀".repeat(8192);
        assert_eq!(decode_field(&value).unwrap(), Some(value));
    }
}
