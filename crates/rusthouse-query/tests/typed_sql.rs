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
fn aggregates_numeric_values_after_text_filters() {
    let part = part();
    for (sql, expected) in [
        (
            "SELECT sum(status) FROM part WHERE service = 'api'",
            "sum(status)\n700\n",
        ),
        (
            "SELECT avg(status) FROM part WHERE service = 'api'",
            "avg(status)\n350\n",
        ),
        (
            "SELECT min(status) FROM part WHERE message IS NOT NULL",
            "min(status)\n200\n",
        ),
        (
            "SELECT max(status) AS peak FROM part WHERE message IS NOT NULL",
            "peak\n500\n",
        ),
        (
            "SELECT varPop(status) FROM part WHERE service = 'api'",
            "varPop(status)\n22500\n",
        ),
        (
            "SELECT stddevPop(status) FROM part WHERE service = 'api'",
            "stddevPop(status)\n150\n",
        ),
        (
            "SELECT median(status) FROM part WHERE service = 'api'",
            "median(status)\n350\n",
        ),
    ] {
        assert_eq!(query(sql, &part), expected, "{sql}");
    }
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn aggregates_ignore_nulls_and_preserve_empty_result_semantics() {
    let part = part();
    for (sql, expected) in [
        ("SELECT sum(attempt) FROM part", "sum(attempt)\n4\n"),
        ("SELECT avg(attempt) FROM part", "avg(attempt)\n2\n"),
        (
            "SELECT min(attempt) FROM part WHERE message IS NULL",
            "min(attempt)\nNULL\n",
        ),
        (
            "SELECT sum(status) FROM part WHERE service = 'missing'",
            "sum(status)\nNULL\n",
        ),
        (
            "SELECT sum(status) FROM part WHERE service = 'api' LIMIT 0",
            "sum(status)\n",
        ),
    ] {
        assert_eq!(query(sql, &part), expected, "{sql}");
    }
    for sql in [
        "SELECT sum(service) FROM part",
        "SELECT sum(missing) FROM part WHERE service = 'api'",
    ] {
        let parsed = parse_select(sql).unwrap();
        assert!(
            execute(&parsed, Some(&part), &mut Vec::new()).is_err(),
            "{sql}"
        );
    }
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn aggregates_typed_filters_across_frozen_parts() {
    let table = std::env::temp_dir().join(format!(
        "rusthouse-typed-aggregates-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&table).unwrap();
    fs::rename(part(), table.join("all_1_1_0")).unwrap();
    fs::rename(part(), table.join("all_2_2_0")).unwrap();
    assert_eq!(
        query("SELECT sum(status) FROM part WHERE service = 'api'", &table),
        "sum(status)\n1400\n"
    );
    assert_eq!(
        query("SELECT sum(attempt) FROM part WHERE service = 'db'", &table),
        "sum(attempt)\nNULL\n"
    );
    fs::remove_dir_all(table).unwrap();
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
