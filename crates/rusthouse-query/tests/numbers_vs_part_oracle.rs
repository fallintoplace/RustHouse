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
                    let expected = expected.into_iter().skip(offset).take(limit).fold(
                        String::from("number\n"),
                        |mut result, value| {
                            result.push_str(&format!("{value}\n"));
                            result
                        },
                    );
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

/// Exercise multiple immutable parts instead of one small in-memory fixture.
fn test_multi_parts(rows: u64) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-query-multi-oracle-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = vec![Column {
        name: "number".into(),
        data_type: "UInt64".into(),
    }];
    for (index, start) in (0..rows).step_by(4096).enumerate() {
        let end = (start + 4096).min(rows);
        let data = (start..end)
            .map(|value| vec![Some(value.to_string())])
            .collect::<Vec<_>>();
        write_wide_part(
            &root.join(format!("all_{index}_{index}_0")),
            &columns,
            &data,
        )
        .unwrap();
    }
    root
}

#[test]
fn multi_part_queries_match_independent_generated_source() {
    // Cross the 1024-row granule and 4096-row part boundaries.
    let rows = 8201;
    let root = test_multi_parts(rows);
    let statements = [
        "SELECT count(*) FROM {source} WHERE number >= 4095",
        "SELECT sum(number) FROM {source} WHERE number >= 4095",
        "SELECT count(DISTINCT number) FROM {source} WHERE number >= 1024",
        "SELECT number FROM {source} WHERE number >= 4096 ORDER BY number DESC LIMIT 17 OFFSET 3",
        "SELECT number FROM {source} WHERE number < 5 OR number >= 8190 ORDER BY number DESC LIMIT 12",
        "SELECT number FROM {source} WHERE number BETWEEN 4093 AND 4102 ORDER BY number ASC LIMIT 5 OFFSET 2",
    ];
    for statement in statements {
        let generated = statement.replace("{source}", &format!("numbers({rows})"));
        let part = statement.replace("{source}", "part");
        assert_eq!(
            run(&part, Some(&root)),
            run(&generated, None),
            "{statement}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stored_count_column_uses_the_same_filter_semantics_as_generated_count() {
    let root = test_multi_parts(8201);
    for predicate in [
        "WHERE number >= 4096",
        "WHERE number % 7 = 0",
        "WHERE number < 25 OR number >= 8190",
        "WHERE number BETWEEN 100 AND 200",
    ] {
        let stored = format!("SELECT count(number) FROM part {predicate}");
        let generated = format!("SELECT count(number) FROM numbers(8201) {predicate}");
        assert_eq!(run(&stored, Some(&root)), run(&generated, None), "{stored}");
    }
    fs::remove_dir_all(root).unwrap();
}
