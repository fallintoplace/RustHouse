use ch_cityhash102::cityhash128;
use rusthouse_query::{execute, parse_select};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn block(bytes: &[u8]) -> Vec<u8> {
    let mut header = vec![0x02];
    header.extend_from_slice(&(9u32 + bytes.len() as u32).to_le_bytes());
    header.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    header.extend_from_slice(bytes);
    let mut payload = u128::from(cityhash128(&header)).to_le_bytes().to_vec();
    payload.extend(header);
    payload
}

fn stream(part: &Path, name: &str, data: &[u8], rows: u64) {
    fs::write(part.join(format!("{name}.bin")), block(data)).unwrap();
    let mut mark = Vec::new();
    mark.extend_from_slice(&0u64.to_le_bytes());
    mark.extend_from_slice(&0u64.to_le_bytes());
    mark.extend_from_slice(&rows.to_le_bytes());
    fs::write(part.join(format!("{name}.mrk2")), mark).unwrap();
}

fn strings(values: &[&str]) -> Vec<u8> {
    let mut result = Vec::new();
    for value in values {
        assert!(value.len() < 128);
        result.push(value.len() as u8);
        result.extend_from_slice(value.as_bytes());
    }
    result
}

fn part() -> PathBuf {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-typed-query-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&part).unwrap();
    fs::write(part.join("count.txt"), "3\n").unwrap();
    fs::write(part.join("columns.txt"),
        "columns format version: 1\n5 columns:\n`service` String\n`status` UInt16\n`message` Nullable(String)\n`uid` UUID\n`attempt` Nullable(UInt16)\n").unwrap();
    stream(&part, "service", &strings(&["api", "db", "api"]), 3);
    stream(
        &part,
        "status",
        &[200u16, 503, 500]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
        3,
    );
    stream(&part, "message", &strings(&["hello", "", "world"]), 3);
    stream(&part, "message.null", &[0, 1, 0], 3);
    stream(&part, "uid", &[0u8; 48], 3);
    stream(
        &part,
        "attempt",
        &[1u16, 999, 3]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
        3,
    );
    stream(&part, "attempt.null", &[0, 1, 0], 3);
    part
}

fn query(sql: &str, part: &Path) -> String {
    let mut result = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(part), &mut result).unwrap();
    String::from_utf8(result).unwrap()
}

#[test]
fn selects_mixed_string_and_numeric_rows() {
    let part = part();
    assert_eq!(
        query(
            "SELECT service, status FROM part WHERE service = 'api' ORDER BY status DESC",
            &part
        ),
        "service\tstatus\napi\t500\napi\t200\n"
    );
    assert_eq!(
        query("SELECT service FROM part WHERE status >= 500", &part),
        "service\ndb\napi\n"
    );
    assert_eq!(
        query("SELECT uid FROM part WHERE service = 'db'", &part),
        "uid\n00000000-0000-0000-0000-000000000000\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn counts_and_filters_nullable_text() {
    let part = part();
    assert_eq!(
        query("SELECT message FROM part WHERE message IS NULL", &part),
        "message\n\\N\n"
    );
    assert_eq!(
        query("SELECT message FROM part WHERE message IS NOT NULL", &part),
        "message\nhello\nworld\n"
    );
    assert_eq!(
        query("SELECT count(*) FROM part WHERE service = 'api'", &part),
        "count\n2\n"
    );
    assert_eq!(
        query("SELECT count(DISTINCT service) FROM part", &part),
        "countDistinct(service)\n2\n"
    );
    assert_eq!(
        query("SELECT DISTINCT service FROM part ORDER BY service", &part),
        "service\napi\ndb\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn rejects_invalid_string_literals_without_panicking() {
    assert!(parse_select("SELECT service FROM part WHERE service = 'unterminated").is_err());
    let part = part();
    assert_eq!(
        query(
            "SELECT service FROM part WHERE service IN ('db', 'api')",
            &part
        ),
        "service\napi\ndb\napi\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn supports_nullable_numeric_values_and_filters() {
    let part = part();
    assert_eq!(
        query("SELECT attempt FROM part WHERE attempt IS NULL", &part),
        "attempt\n\\N\n"
    );
    assert_eq!(
        query("SELECT attempt FROM part WHERE attempt > 2", &part),
        "attempt\n3\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn skips_end_of_file_terminal_zero_row_marks() {
    use std::io::Write;
    let part = part();
    for name in [
        "service",
        "status",
        "message",
        "message.null",
        "uid",
        "attempt",
        "attempt.null",
    ] {
        let bytes = fs::metadata(part.join(format!("{name}.bin")))
            .unwrap()
            .len();
        let mut mark = fs::OpenOptions::new()
            .append(true)
            .open(part.join(format!("{name}.mrk2")))
            .unwrap();
        mark.write_all(&bytes.to_le_bytes()).unwrap();
        mark.write_all(&0u64.to_le_bytes()).unwrap();
        mark.write_all(&0u64.to_le_bytes()).unwrap();
    }
    assert_eq!(
        query(
            "SELECT service, status FROM part ORDER BY status DESC LIMIT 2",
            &part
        ),
        "service\tstatus\ndb\t503\napi\t500\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn filters_text_by_like_and_not_like_across_nullable_values() {
    let part = part();
    assert_eq!(
        query("SELECT service FROM part WHERE service LIKE 'a_i'", &part),
        "service\napi\napi\n"
    );
    assert_eq!(
        query(
            "SELECT service FROM part WHERE service NOT LIKE 'a%'",
            &part
        ),
        "service\ndb\n"
    );
    assert_eq!(
        query(
            "SELECT message FROM part WHERE message NOT LIKE '%l%'",
            &part
        ),
        "message\n"
    );
    assert_eq!(
        query("SELECT message FROM part WHERE message LIKE 'w%'", &part),
        "message\nworld\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn rejects_like_for_numeric_columns() {
    let part = part();
    let mut output = Vec::new();
    let query = parse_select("SELECT service FROM part WHERE status LIKE '5%'").unwrap();
    assert!(execute(&query, Some(&part), &mut output).is_err());

    let number_query =
        parse_select("SELECT number FROM numbers(10) WHERE number LIKE '1%'").unwrap();
    assert!(execute(&number_query, None, &mut output).is_err());
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn queries_typed_columns_across_frozen_parts() {
    let table = std::env::temp_dir().join(format!(
        "rusthouse-typed-multipart-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&table).unwrap();
    fs::rename(part(), table.join("all_1_1_0")).unwrap();
    fs::rename(part(), table.join("all_2_2_0")).unwrap();
    assert_eq!(
        query("SELECT count(*) FROM part WHERE service = 'api'", &table),
        "count\n4\n"
    );
    assert_eq!(
        query(
            "SELECT service, status FROM part WHERE status > 500 ORDER BY status DESC LIMIT 2",
            &table
        ),
        "service\tstatus\ndb\t503\ndb\t503\n"
    );
    assert_eq!(
        query("SELECT DISTINCT service FROM part ORDER BY service", &table),
        "service\napi\ndb\n"
    );
    fs::remove_dir_all(table).unwrap();
}

#[test]
fn groups_typed_columns_with_filters_having_and_pagination() {
    let part = part();
    assert_eq!(
        query(
            "SELECT service, count(*) FROM part GROUP BY service ORDER BY count DESC",
            &part
        ),
        "service\tcount\napi\t2\ndb\t1\n"
    );
    assert_eq!(
        query(
            "SELECT service, count(*) FROM part WHERE status >= 500 GROUP BY service ORDER BY service DESC",
            &part
        ),
        "service\tcount\ndb\t1\napi\t1\n"
    );
    assert_eq!(
        query(
            "SELECT service, count(*) FROM part GROUP BY service HAVING count(*) >= 2 ORDER BY count DESC LIMIT 1",
            &part
        ),
        "service\tcount\napi\t2\n"
    );
    assert_eq!(
        query(
            "SELECT service, count(*) FROM part GROUP BY service ORDER BY count DESC LIMIT 1 OFFSET 1",
            &part
        ),
        "service\tcount\ndb\t1\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn groups_nullable_and_uuid_keys_including_null() {
    let part = part();
    assert_eq!(
        query(
            "SELECT message, count(*) FROM part GROUP BY message ORDER BY message",
            &part
        ),
        "message\tcount\n\\N\t1\nhello\t1\nworld\t1\n"
    );
    assert_eq!(
        query(
            "SELECT attempt, count(*) FROM part GROUP BY attempt ORDER BY attempt",
            &part
        ),
        "attempt\tcount\n\\N\t1\n1\t1\n3\t1\n"
    );
    assert_eq!(
        query("SELECT uid, count(*) FROM part GROUP BY uid", &part),
        "uid\tcount\n00000000-0000-0000-0000-000000000000\t3\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn matches_ilike_on_string_and_nullable_columns() {
    let part = part();
    assert_eq!(
        query("SELECT service FROM part WHERE service ILIKE 'A_I'", &part),
        "service\napi\napi\n"
    );
    assert_eq!(
        query("SELECT service FROM part WHERE service NOT ILIKE 'API'", &part),
        "service\ndb\n"
    );
    assert_eq!(
        query(
            "SELECT service FROM part WHERE service ILIKE 'API' OR service LIKE 'd_'",
            &part
        ),
        "service\napi\ndb\napi\n"
    );
    assert_eq!(
        query("SELECT message FROM part WHERE message NOT ILIKE '%O%'", &part),
        "message\n"
    );
    let mut output = Vec::new();
    let numeric = parse_select("SELECT status FROM part WHERE status ILIKE '5%'").unwrap();
    assert!(execute(&numeric, Some(&part), &mut output).is_err());
    fs::remove_dir_all(part).unwrap();
}

