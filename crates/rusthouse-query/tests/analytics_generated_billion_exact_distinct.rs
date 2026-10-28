//! Exact mixed COUNT(DISTINCT) on billion-row virtual analytics input.
use rusthouse_query::{execute, parse_select};

fn result(sql: &str) -> String {
    let mut out = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn billion_row_distinct_count_is_exact_without_a_hash_set() {
    assert_eq!(
        result("SELECT count(DISTINCT number), sum(number), count(*) FROM numbers(1000000000)"),
        "count(DISTINCT number)\tsum(number)\tcount(*)\n1000000000\t499999999500000000\t1000000000\n"
    );
}
#[test]
fn billion_row_filtered_distinct_stays_exact() {
    assert_eq!(
        result("SELECT count(DISTINCT number), min(number), max(number) FROM numbers(1000000000) WHERE number >= 999000000"),
        "count(DISTINCT number)\tmin(number)\tmax(number)\n1000000\t999000000\t999999999\n"
    );
}
#[test]
fn empty_and_strided_generated_series_have_exact_distinct_counts() {
    assert_eq!(
        result("SELECT count(DISTINCT number), count(*) FROM numbers(0)"),
        "count(DISTINCT number)\tcount(*)\n0\t0\n"
    );
    assert_eq!(
        result("SELECT count(DISTINCT number), sum(number) FROM generate_series(2, 12, 2)"),
        "count(DISTINCT number)\tsum(number)\n6\t42\n"
    );
}
