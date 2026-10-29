use super::{Config, LimitedWriter, Response, write_json_string};
use rusthouse_query::{Ddl, parse_ddl};
use rusthouse_storage::acquire_database_lock;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

const MAX_TABLES: usize = 1024;
const MAX_DIRECTORY_ENTRIES: usize = 10_000;
const MAX_SCHEMA_BYTES: u64 = 32 * 1024;

fn identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 128
        && matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn read_schema_bytes(path: &Path) -> Option<Vec<u8>> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return None;
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_SCHEMA_BYTES {
        return None;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return None;
    };
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    if Read::by_ref(&mut file)
        .take(MAX_SCHEMA_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_SCHEMA_BYTES
    {
        return None;
    }
    Some(bytes)
}

fn valid_schema(path: &Path) -> bool {
    let Some(bytes) = read_schema_bytes(path) else {
        return false;
    };
    std::str::from_utf8(&bytes).is_ok_and(valid_schema_contents)
}

fn valid_schema_line<'a>(line: &'a str, names: &mut HashSet<&'a str>) -> bool {
    let Some((name, data_type)) = line.split_once('\t') else {
        return false;
    };
    let ddl = format!("CREATE TABLE _check ({name} {data_type})");
    identifier(name)
        && names.insert(name)
        && matches!(parse_ddl(&ddl), Ok(Some(Ddl::CreateTable { .. })))
}

fn valid_schema_contents(contents: &str) -> bool {
    if !contents.ends_with('\n') {
        return false;
    }
    let mut lines = contents.lines();
    if lines.next() != Some("rusthouse-schema-v1") {
        return false;
    }
    let mut names = HashSet::new();
    let mut count = 0;
    for line in lines {
        count += 1;
        if count > 64 {
            return false;
        }
        if !valid_schema_line(line, &mut names) {
            return false;
        }
    }
    count > 0
}

fn discoverable_table(database: &Path, name: &str, schema: &Path) -> bool {
    if !identifier(name) || !valid_schema(schema) {
        return false;
    }
    let parts = database.join(format!("{name}.parts"));
    fs::symlink_metadata(&parts).is_ok_and(|metadata| metadata.file_type().is_dir())
}

pub(super) fn table_names(database: &Path) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let entries = fs::read_dir(database)
        .map_err(|error| format!("cannot read database directory: {error}"))?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES {
            return Err("database directory exceeds 10000 entries".into());
        }
        let entry = entry.map_err(|error| format!("cannot read database entry: {error}"))?;
        let file_name = entry.file_name();
        let Some(name) = file_name
            .to_str()
            .and_then(|name| name.strip_suffix(".schema"))
        else {
            continue;
        };
        if !discoverable_table(database, name, &entry.path()) {
            continue;
        }
        if result.len() == MAX_TABLES {
            return Err("database exceeds the 1024-table discovery limit".into());
        }
        result.push(name.to_owned());
    }
    result.sort_unstable();
    Ok(result)
}

fn write_table_listing<W: Write>(output: &mut W, names: &[String]) -> io::Result<()> {
    output.write_all(b"{\"tables\":[")?;
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            output.write_all(b",")?;
        }
        write_json_string(output, name)?;
    }
    output.write_all(b"]}\n")
}

pub(super) fn handle(method: &str, body: &[u8], config: &Config) -> Response {
    if method != "GET" {
        return Response::text("405 Method Not Allowed", "tables endpoint requires GET\n");
    }
    if !body.is_empty() {
        return Response::text("400 Bad Request", "GET must not contain a request body\n");
    }
    if config.part.is_some() {
        return Response::text(
            "400 Bad Request",
            "tables endpoint requires --database only\n",
        );
    }
    let Some(database) = config.database.as_deref() else {
        return Response::text("400 Bad Request", "no --database source configured\n");
    };
    let _lock = match acquire_database_lock(database, false) {
        Ok(lock) => lock,
        Err(error) => return Response::text("409 Conflict", &format!("{error}\n")),
    };
    let names = match table_names(database) {
        Ok(names) => names,
        Err(error) => return Response::text("400 Bad Request", &format!("{error}\n")),
    };
    let mut output = LimitedWriter::new();
    let write = write_table_listing(&mut output, &names);
    match write {
        Ok(()) => Response {
            status: "200 OK",
            content_type: "application/json; charset=utf-8",
            body: output.data,
        },
        Err(_) => Response::text("413 Payload Too Large", "table listing exceeds 8 MiB\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

    fn temporary_database() -> std::path::PathBuf {
        let db = std::env::temp_dir().join(format!(
            "rusthouse-tables-{}-{}",
            std::process::id(),
            NEXT_DB.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&db).unwrap();
        db
    }

    #[test]
    fn serializes_empty_and_escaped_table_lists() {
        let mut output = Vec::new();
        write_table_listing(&mut output, &[]).unwrap();
        assert_eq!(output, b"{\"tables\":[]}\n");

        output.clear();
        write_table_listing(&mut output, &["a\"b".into(), "c".into()]).unwrap();
        assert_eq!(output, b"{\"tables\":[\"a\\\"b\",\"c\"]}\n");
    }

    #[test]
    fn validates_schema_contents_independently_of_catalog_io() {
        assert!(valid_schema_contents("rusthouse-schema-v1\nid\tUInt64\n"));
        assert!(!valid_schema_contents("rusthouse-schema-v1\n"));
        assert!(!valid_schema_contents("rusthouse-schema-v1\nid\tUInt64"));
        assert!(!valid_schema_contents("wrong-version\nid\tUInt64\n"));
        assert!(!valid_schema_contents("rusthouse-schema-v1\nno-type\n"));
        assert!(!valid_schema_contents(
            "rusthouse-schema-v1\nid\tUnknownType\n"
        ));
        assert!(!valid_schema_contents(
            "rusthouse-schema-v1\nid\tUInt64 junk\n"
        ));
        assert!(!valid_schema_contents(
            "rusthouse-schema-v1\nid\tUInt64\nid\tString\n"
        ));
    }

    #[test]
    fn lists_only_valid_catalog_tables_in_sorted_order() {
        let db = temporary_database();
        for table in ["users", "events"] {
            fs::write(
                db.join(format!("{table}.schema")),
                "rusthouse-schema-v1\nid\tUInt64\n",
            )
            .unwrap();
            fs::create_dir(db.join(format!("{table}.parts"))).unwrap();
        }
        fs::write(db.join("broken.schema"), "invalid\n").unwrap();
        fs::create_dir(db.join("broken.parts")).unwrap();
        fs::write(
            db.join("orphan.schema"),
            "rusthouse-schema-v1\nid\tUInt64\n",
        )
        .unwrap();
        let config = Config {
            database: Some(db.clone()),
            ..Config::default()
        };
        let result = crate::handle_request("GET", "/tables", b"", &config);
        assert_eq!(result.status, "200 OK");
        assert_eq!(result.content_type, "application/json; charset=utf-8");
        assert_eq!(result.body, br#"{"tables":["events","users"]}"#.iter().copied()
            .chain(b"\n".iter().copied()).collect::<Vec<_>>());
        assert_eq!(
            crate::handle_request("POST", "/tables", b"", &config).status,
            "405 Method Not Allowed"
        );
        assert_eq!(
            crate::handle_request("GET", "/tables", b"ignored", &config).status,
            "400 Bad Request"
        );
        fs::remove_dir_all(db).unwrap();
    }


    #[test]
    fn clickhouse_show_tables_reads_only_valid_catalog_and_supports_like() {
        let db = temporary_database();
        for name in ["events", "event_log", "users"] {
            fs::write(
                db.join(format!("{name}.schema")),
                "rusthouse-schema-v1\nid\tUInt64\n",
            )
            .unwrap();
            fs::create_dir(db.join(format!("{name}.parts"))).unwrap();
        }
        fs::write(db.join("broken.schema"), "invalid\n").unwrap();
        fs::create_dir(db.join("broken.parts")).unwrap();
        let config = Config {
            database: Some(db.clone()),
            max_rows: 10,
            ..Config::default()
        };
        let response = crate::handle_request("POST", "/", b"SHOW TABLES FORMAT TSV", &config);
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.body, b"event_log\nevents\nusers\n");
        let filtered = crate::handle_request(
            "POST",
            "/?default_format=TSV",
            b"SHOW TABLES LIKE 'event%'",
            &config,
        );
        assert_eq!(filtered.status, "200 OK");
        assert_eq!(filtered.body, b"event_log\nevents\n");
        let excluded = crate::handle_request(
            "POST",
            "/",
            b"SHOW TABLES NOT LIKE 'event%' FORMAT TSV",
            &config,
        );
        assert_eq!(excluded.status, "200 OK");
        assert_eq!(excluded.body, b"users\n");
        assert_eq!(
            crate::handle_request("POST", "/", b"SHOW DATABASES", &config).status,
            "400 Bad Request"
        );
        assert_eq!(
            crate::handle_request("POST", "/", b"DROP TABLE users", &config).status,
            "400 Bad Request"
        );
        fs::remove_dir_all(db).unwrap();
    }

    #[test]
    fn requires_exclusive_database_mode() {
        let empty = crate::handle_request("GET", "/tables", b"", &Config::default());
        assert_eq!(empty.status, "400 Bad Request");
        let part = crate::handle_request(
            "GET",
            "/tables",
            b"",
            &Config {
                part: Some("part".into()),
                ..Config::default()
            },
        );
        assert_eq!(part.status, "400 Bad Request");
    }
}
