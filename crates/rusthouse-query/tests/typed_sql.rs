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
fn limit_zero_does_not_open_typed_data_streams() {
    let part = part();
    fs::remove_file(part.join("service.bin")).unwrap();
    for (sql, expected) in [
        ("SELECT service FROM part LIMIT 0", "service\n"),
        (
            "SELECT sum(status) FROM part WHERE service = 'api' LIMIT 0",
            "sum(status)\n",
        ),
        (
            "SELECT service, sum(status) FROM part GROUP BY service LIMIT 0",
            "service\tsum(status)\n",
        ),
    ] {
        assert_eq!(query(sql, &part), expected, "{sql}");
    }
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn count_column_excludes_nulls_after_typed_filters() {
    let part = part();
    for (sql, expected) in [
        ("SELECT count(*) FROM part", "count\n3\n"),
        ("SELECT count(attempt) FROM part", "count(attempt)\n2\n"),
        (
            "SELECT count(message) FROM part WHERE service = 'api'",
            "count(message)\n2\n",
        ),
        (
            "SELECT count(attempt) FROM part WHERE service = 'db'",
            "count(attempt)\n0\n",
        ),
        ("SELECT count(uid) AS total FROM part", "total\n3\n"),
        (
            "SELECT count(attempt) FROM part WHERE message IS NULL",
            "count(attempt)\n0\n",
        ),
        (
            "SELECT count(attempt) FROM part LIMIT 0",
            "count(attempt)\n",
        ),
        (
            "SELECT count(attempt) FROM part LIMIT 1 OFFSET 1",
            "count(attempt)\n",
        ),
    ] {
        assert_eq!(query(sql, &part), expected, "{sql}");
    }
    let invalid = parse_select("SELECT count(unknown) FROM part").unwrap();
    assert!(execute(&invalid, Some(&part), &mut Vec::new()).is_err());
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn fused_count_columns_respects_string_uuid_and_nullable_null_maps() {
    let part = part();
    let all = "SELECT count(service), count(message), count(uid), count(attempt) FROM part";
    let api = concat!(
        "SELECT count(service), count(message), count(uid), count(attempt) ",
        "FROM part WHERE service = 'api'"
    );
    assert_eq!(
        query(all, &part),
        "count(service)\tcount(message)\tcount(uid)\tcount(attempt)\n3\t2\t3\t2\n"
    );
    assert_eq!(
        query(api, &part),
        "count(service)\tcount(message)\tcount(uid)\tcount(attempt)\n2\t2\t2\t2\n"
    );
    fs::remove_dir_all(part).unwrap();
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
fn length_aggregates_reuse_string_spans_and_skip_nulls() {
    let part = part();
    for (sql, expected) in [
        (
            "SELECT sum(length(service)), avg(length(message)) FROM part",
            "sum(length(service))\tavg(length(message))\n8\t5\n",
        ),
        (
            "SELECT sum(length(message)), count(message) FROM part WHERE status >= 500",
            "sum(length(message))\tcount(message)\n5\t1\n",
        ),
        (
            "SELECT sum(length(message)), count(message) FROM part WHERE message IS NULL",
            "sum(length(message))\tcount(message)\nNULL\t0\n",
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
fn typed_offset_skips_filtered_rows_without_changing_distinct_semantics() {
    let part = part();
    assert_eq!(
        query(
            "SELECT message FROM part WHERE service = 'api' LIMIT 1 OFFSET 1",
            &part
        ),
        "message\nworld\n"
    );
    assert_eq!(
        query("SELECT service FROM part LIMIT 1 OFFSET 2", &part),
        "service\napi\n"
    );
    assert_eq!(
        query("SELECT DISTINCT service FROM part LIMIT 1 OFFSET 1", &part),
        "service\ndb\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn typed_topk_keeps_stable_ties_and_respects_offsets() {
    let part = part();
    assert_eq!(
        query(
            "SELECT service FROM part ORDER BY status DESC LIMIT 1 OFFSET 1",
            &part
        ),
        "service\napi\n"
    );
    assert_eq!(
        query(
            "SELECT status FROM part ORDER BY service ASC LIMIT 2",
            &part
        ),
        "status\n200\n500\n"
    );
    assert_eq!(
        query(
            "SELECT status FROM part ORDER BY service ASC LIMIT 1 OFFSET 1",
            &part
        ),
        "status\n500\n"
    );
    assert_eq!(
        query(
            "SELECT status FROM part ORDER BY service DESC LIMIT 2",
            &part
        ),
        "status\n503\n200\n"
    );
    assert_eq!(
        query(
            "SELECT status FROM part ORDER BY service ASC, status DESC LIMIT 2",
            &part
        ),
        "status\n500\n200\n"
    );
    fs::remove_dir_all(part).unwrap();
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
fn wildcard_rename_works_with_typed_parts_and_rejects_name_collisions() {
    let part = part();
    assert_eq!(
        query(
            "SELECT * RENAME service AS origin FROM part WHERE service = 'db'",
            &part
        ),
        "origin\tstatus\tmessage\tuid\tattempt\ndb\t503\t\\N\t00000000-0000-0000-0000-000000000000\t\\N\n"
    );
    assert_eq!(
        query(
            "SELECT * RENAME status AS code FROM part ORDER BY code DESC LIMIT 1",
            &part
        ),
        "service\tcode\tmessage\tuid\tattempt\ndb\t503\t\\N\t00000000-0000-0000-0000-000000000000\t\\N\n"
    );
    for sql in [
        "SELECT * RENAME service AS status FROM part",
        "SELECT * RENAME missing AS renamed FROM part LIMIT 0",
    ] {
        let select = parse_select(sql).unwrap();
        let mut output = Vec::new();
        assert!(execute(&select, Some(&part), &mut output).is_err(), "{sql}");
        assert!(output.is_empty(), "{sql}");
    }
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
        query("SELECT count(DISTINCT message) FROM part", &part),
        "countDistinct(message)\n2\n"
    );
    assert_eq!(
        query("SELECT count(DISTINCT attempt) FROM part", &part),
        "countDistinct(attempt)\n2\n"
    );
    // COUNT(DISTINCT) must not count NULLs or repeated text values.
    assert_eq!(
        query("SELECT count(DISTINCT uid) FROM part", &part),
        "countDistinct(uid)\n1\n"
    );
    assert_eq!(
        query("SELECT count(DISTINCT message) FROM part WHERE message IS NULL", &part),
        "countDistinct(message)\n0\n"
    );
    assert_eq!(
        query("SELECT count(DISTINCT service) FROM part WHERE status > 200", &part),
        "countDistinct(service)\n2\n"
    );
    assert_eq!(
        query("SELECT count(DISTINCT status) FROM part WHERE service = 'api'", &part),
        "countDistinct(status)\n2\n"
    );
    assert_eq!(
        query("SELECT count(DISTINCT service) FROM part LIMIT 0", &part),
        "countDistinct(service)\n"
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
fn uuid_comparisons_and_ranges_use_canonical_byte_order() {
    let part = part();
    let mut raw = [0u8; 48];
    raw[0] = 1;
    raw[16] = 2;
    raw[32] = 3;
    stream(&part, "uid", &raw, 3);
    for (sql, expected) in [
        (
            "SELECT service FROM part WHERE uid = '00000000-0000-0002-0000-000000000000'",
            "service\ndb\n",
        ),
        (
            "SELECT count(*) FROM part WHERE uid >= '00000000-0000-0002-0000-000000000000'",
            "count\n2\n",
        ),
        (
            "SELECT count(*) FROM part WHERE uid NOT BETWEEN '00000000-0000-0001-0000-000000000000' AND '00000000-0000-0002-0000-000000000000'",
            "count\n1\n",
        ),
        (
            "SELECT count(*) FROM part WHERE uid != '00000000-0000-0003-0000-000000000000'",
            "count\n2\n",
        ),
    ] {
        assert_eq!(query(sql, &part), expected, "{sql}");
    }
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
fn distinct_uuid_counts_duplicate_binary_values_once() {
    let part = part();
    let mut raw = [0u8; 48];
    raw[0] = 1;
    raw[16] = 1;
    raw[32] = 2;
    stream(&part, "uid", &raw, 3);
    for (sql, expected) in [
        ("SELECT count(DISTINCT uid) FROM part", "countDistinct(uid)\n2\n"),
        (
            "SELECT count(DISTINCT uid) FROM part WHERE service = 'api'",
            "countDistinct(uid)\n2\n",
        ),
        (
            "SELECT count(DISTINCT uid) FROM part WHERE service = 'db'",
            "countDistinct(uid)\n1\n",
        ),
        (
            "SELECT count(DISTINCT uid) FROM part LIMIT 0",
            "countDistinct(uid)\n",
        ),
    ] {
        assert_eq!(query(sql, &part), expected, "{sql}");
    }
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
fn groups_numeric_aggregates_by_typed_keys() {
    let part = part();
    assert_eq!(
        query(
            "SELECT service, sum(status) FROM part GROUP BY service ORDER BY service",
            &part,
        ),
        "service\tsum(status)\napi\t700\ndb\t503\n"
    );
    assert_eq!(
        query(
            "SELECT service, avg(status) FROM part GROUP BY service ORDER BY service",
            &part,
        ),
        "service\tavg(status)\napi\t350\ndb\t503\n"
    );
    assert_eq!(
        query(
            "SELECT service, sum(attempt) FROM part GROUP BY service ORDER BY service",
            &part,
        ),
        "service\tsum(attempt)\napi\t4\ndb\tNULL\n"
    );
    assert_eq!(
        query(
            "SELECT message, min(status) FROM part GROUP BY message ORDER BY message",
            &part,
        ),
        "message\tmin(status)\n\\N\t503\nhello\t200\nworld\t500\n"
    );
    assert_eq!(
        query(
            "SELECT service, max(status) FROM part WHERE status >= 500 GROUP BY service ORDER BY service",
            &part,
        ),
        "service\tmax(status)\napi\t500\ndb\t503\n"
    );
    let invalid = parse_select("SELECT service, sum(message) FROM part GROUP BY service").unwrap();
    let mut output = Vec::new();
    assert!(execute(&invalid, Some(&part), &mut output).is_err());
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn grouped_numeric_aggregates_respect_explicit_null_precedence() {
    let part = part();
    assert_eq!(
        query(
            "SELECT message, min(status), max(status) FROM part GROUP BY message ORDER BY message ASC NULLS LAST",
            &part,
        ),
        "message\tmin(status)\tmax(status)\nhello\t200\t200\nworld\t500\t500\n\\N\t503\t503\n"
    );
    assert_eq!(
        query(
            "SELECT message, min(status), max(status) FROM part GROUP BY message ORDER BY message DESC NULLS FIRST",
            &part,
        ),
        "message\tmin(status)\tmax(status)\n\\N\t503\t503\nworld\t500\t500\nhello\t200\t200\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn orders_nullable_columns_with_explicit_null_precedence() {
    let part = part();
    assert_eq!(
        query(
            "SELECT attempt FROM part ORDER BY attempt ASC NULLS LAST",
            &part
        ),
        "attempt\n1\n3\n\\N\n"
    );
    assert_eq!(
        query(
            "SELECT attempt FROM part ORDER BY attempt DESC NULLS FIRST",
            &part
        ),
        "attempt\n\\N\n3\n1\n"
    );
    assert_eq!(
        query(
            "SELECT message, count(*) FROM part GROUP BY message ORDER BY message DESC NULLS FIRST",
            &part,
        ),
        "message\tcount\n\\N\t1\nworld\t1\nhello\t1\n"
    );
    assert_eq!(
        query(
            "SELECT attempt FROM part ORDER BY attempt ASC NULLS FIRST",
            &part
        ),
        "attempt\n\\N\n1\n3\n"
    );
    assert_eq!(
        query(
            "SELECT attempt FROM part ORDER BY attempt DESC NULLS LAST",
            &part
        ),
        "attempt\n3\n1\n\\N\n"
    );
    let sql = "SELECT message, attempt FROM part ORDER BY attempt ASC NULLS LAST, message DESC";
    let whole = query(sql, &part);
    assert_eq!(whole, "message\tattempt\nhello\t1\nworld\t3\n\\N\t\\N\n");
    let limited = query(&format!("{sql} LIMIT 2"), &part);
    assert_eq!(limited, "message\tattempt\nhello\t1\nworld\t3\n");
    assert_eq!(
        limited,
        format!("{}\n", whole.lines().take(3).collect::<Vec<_>>().join("\n"))
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
        query(
            "SELECT service FROM part WHERE service NOT ILIKE 'API'",
            &part
        ),
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
        query(
            "SELECT message FROM part WHERE message NOT ILIKE '%O%'",
            &part
        ),
        "message\n"
    );
    let mut output = Vec::new();
    let numeric = parse_select("SELECT status FROM part WHERE status ILIKE '5%'").unwrap();
    assert!(execute(&numeric, Some(&part), &mut output).is_err());
    fs::remove_dir_all(part).unwrap();
}
