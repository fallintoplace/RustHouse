//! INSERT ... SELECT reuses the query executor and the validated INSERT publisher.
//! Result decoding is bounded and keeps RustHouse's escaped TSV NULL semantics.
use rusthouse_query::{Insert, InsertSelect, Source, execute};
use std::io::{self, Write};
use std::path::Path;

const MAX_INSERT_ROWS: usize = 4096;
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct BoundedOutput(Vec<u8>);

impl Write for BoundedOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > MAX_RESULT_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("INSERT SELECT result exceeds 8 MiB"));
        }
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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

fn decode_rows(data: &[u8]) -> Result<Vec<Vec<Option<String>>>, String> {
    let value = std::str::from_utf8(data).map_err(|_| "invalid UTF-8 in SELECT result")?;
    if !value.ends_with('\n') {
        return Err("incomplete SELECT result".into());
    }
    let mut lines = value.split_terminator('\n');
    lines.next().ok_or("missing SELECT result header")?;
    let mut rows = Vec::new();
    for line in lines {
        if rows.len() >= MAX_INSERT_ROWS {
            return Err("INSERT SELECT supports at most 4096 rows".into());
        }
        let fields = line
            .split('\t')
            .map(decode_field)
            .collect::<Result<Vec<_>, _>>()?;
        rows.push(fields);
    }
    Ok(rows)
}

pub(super) fn run(database: &Path, statement: &InsertSelect) -> Result<(), String> {
    // Keep source resolution, SELECT execution and final append under one
    // database write lock so concurrent RENAME/DROP cannot invalidate the
    // input, and self-INSERT observes a consistent pre-insert snapshot.
    crate::database::with_write_lock(database, || {
        let mut query = statement.select.clone();
        // Read one extra row to reject truncation rather than silently losing rows.
        let cap = MAX_INSERT_ROWS + 1;
        query.limit = Some(query.limit.map_or(cap, |limit| limit.min(cap)));
        let part = if let Some(Source::Table(table)) = query.source.as_ref() {
            let path = crate::database::select_source(database, table)?;
            query.source = Some(Source::Table("part".into()));
            Some(path)
        } else {
            None
        };
        let mut output = BoundedOutput::default();
        execute(&query, part.as_deref(), &mut output).map_err(|error| error.to_string())?;
        let rows = decode_rows(&output.0)?;
        crate::database::insert_locked(
            database,
            &Insert {
                table: statement.table.clone(),
                columns: statement.columns.clone(),
                rows,
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding_preserves_nulls_and_escaped_text() {
        let rows =
            decode_rows(b"id\ttext\tnote\n1\tline\\nnext\t\\N\n2\t\\\\N\tback\\\\slash\n").unwrap();
        assert_eq!(rows[0][1], Some("line\nnext".into()));
        assert_eq!(rows[0][2], None);
        assert_eq!(rows[1][1], Some(r"\N".into()));
        assert_eq!(rows[1][2], Some(r"back\slash".into()));
        assert!(decode_rows(b"id\n1").is_err());
        assert!(decode_rows(b"id\n\\xZ1\n").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn locks_before_resolving_a_source_table() {
        use std::fs;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let database = std::env::temp_dir().join(format!(
            "rusthouse-insert-select-lock-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let statement = rusthouse_query::parse_insert_select(
            "INSERT INTO target SELECT id FROM missing",
        )
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

    #[test]
    fn bounded_result_never_grows_past_limit() {
        let mut output = BoundedOutput(vec![0; MAX_RESULT_BYTES]);
        assert!(output.write_all(b"additional").is_err());
        assert_eq!(output.0.len(), MAX_RESULT_BYTES);
    }
}
