//! Differential correctness checks against an independent Rust reference
//! and the generated numbers() path. No ClickHouse server is required.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn test_part(rows: u64) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-query-oracle-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = vec![Column {
        name: "number".into(),
        data_type: "UInt64".into(),
    }];
    let data = (0..rows)
        .map(|row| vec![Some(row.to_string())])
        .collect::<Vec<_>>();
    write_wide_part(&root.join("all_1_1_0"), &columns, &data).unwrap();
    root
}

fn run(sql: &str, source: Option<&std::path::Path>) -> String {
    let query = parse_select(sql).unwrap();
    let mut result = Vec::new();
    execute(&query, source, &mut result).unwrap();
    String::from_utf8(result).unwrap()
}

#[test]
fn numeric_predicates_ordering_and_pagination_match_written_parts() {
    let rows = 2049;
    let root = test_part(rows);
    for cutoff in [0, 1, 17, 2047, 3000] {
        for modulus in [2, 3, 7] {
            for descending in [false, true] {
                for (offset, limit) in [(0, 1), (0, 10), (2, 5), (20, 100)] {
                    let direction = if descending { "DESC" } else { "ASC" };
                    let predicate = format!(
                        "WHERE number >= {cutoff} AND number % {modulus} = 0                          ORDER BY number {direction} LIMIT {limit} OFFSET {offset}"
                    );
                    let generated_sql = format!("SELECT number FROM numbers({rows}) {predicate}");
                    let parts_sql = format!("SELECT number FROM part {predicate}");
                    let mut expected = (0..rows)
                        .filter(|value| *value >= cutoff && *value % modulus == 0)
                        .collect::<Vec<_>>();
                    if descending {
                        expected.reverse();
                    }
                    let expected = expected
                        .into_iter()
                        .skip(offset)
                        .take(limit)
                        .fold(String::from("number\n"), |mut result, value| {
                            result.push_str(&format!("{value}\n"));
                            result
                        });
                    assert_eq!(run(&generated_sql, None), expected, "{generated_sql}");
                    assert_eq!(run(&parts_sql, Some(&root)), expected, "{parts_sql}");
                }
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn null_and_empty_result_queries_agree_between_sources() {
    let root = test_part(0);
    for sql in [
        "SELECT count(*) FROM {source}",
        "SELECT number FROM {source} LIMIT 0",
        "SELECT number FROM {source} WHERE number >= 10 LIMIT 10",
    ] {
        let generated = sql.replace("{source}", "numbers(0)");
        let part = sql.replace("{source}", "part");
        assert_eq!(run(&generated, None), run(&part, Some(&root)), "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
