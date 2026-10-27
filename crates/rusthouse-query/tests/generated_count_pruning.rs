use rusthouse_query::{execute, parse_select};

fn result(sql: &str) -> String {
    let parsed = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&parsed, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn counts_filtered_large_generated_ranges_without_scanning() {
    for (sql, expected) in [
        (
            "SELECT count(*) FROM numbers(1000000) WHERE number >= 999995 AND number <= 999998",
            "count\n4\n",
        ),
        (
            "SELECT count(number) FROM numbers(1000000) WHERE number < 4 AND number != 1",
            "count(number)\n3\n",
        ),
        (
            "SELECT count(DISTINCT number) FROM numbers(1000000) WHERE number = 999999",
            "countDistinct(number)\n1\n",
        ),
        (
            "SELECT count(*) FROM numbers(10) WHERE number > 100000000000000000000",
            "count\n0\n",
        ),
        (
            "SELECT count(*) FROM numbers(10) WHERE number < 100000000000000000000",
            "count\n10\n",
        ),
        (
            "SELECT count(*) FROM numbers(0) WHERE number >= 0",
            "count\n0\n",
        ),
    ] {
        assert_eq!(result(sql), expected, "{sql}");
    }
}

#[test]
fn respects_steps_and_unsigned_boundaries() {
    assert_eq!(
        result(
            "SELECT count(*) FROM numbers(12, 31, 5) WHERE number >= 22 AND number < 40 AND number != 27"
        ),
        "count\n3\n"
    );
    assert_eq!(
        result(
            "SELECT count(number) FROM generate_series(18446744073709551613, 18446744073709551615) WHERE number >= 18446744073709551614"
        ),
        "count(number)\n2\n"
    );
    assert_eq!(
        result(
            "SELECT count(*) FROM numbers(18446744073709551615, 1) WHERE number <= 18446744073709551615"
        ),
        "count\n1\n"
    );
}

#[test]
fn retains_rowwise_fallback_for_or_modulo_and_in() {
    for (sql, expected) in [
        (
            "SELECT count(*) FROM numbers(10) WHERE number = 1 OR number = 3",
            "count\n2\n",
        ),
        (
            "SELECT count(*) FROM numbers(10) WHERE number % 3 = 1",
            "count\n3\n",
        ),
        (
            "SELECT count(*) FROM numbers(10) WHERE number IN (1, 2, 3)",
            "count\n3\n",
        ),
    ] {
        assert_eq!(result(sql), expected, "{sql}");
    }
}
