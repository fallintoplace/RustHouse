use rusthouse_query::{execute, parse_select};

fn output(sql: &str) -> String {
    let mut result = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut result).unwrap();
    String::from_utf8(result).unwrap()
}

#[test]
fn billion_row_sums_and_extrema_use_exact_generated_intervals() {
    for (sql, expected) in [
        (
            "SELECT sum(number) FROM numbers(1000000000) WHERE number >= 999999994 AND number < 999999998",
            "sum(number)\n3999999982\n",
        ),
        (
            "SELECT min(number) FROM numbers(1000000000) WHERE number >= 999999994 AND number < 999999998",
            "min(number)\n999999994\n",
        ),
        (
            "SELECT max(number) FROM numbers(1000000000) WHERE number >= 999999994 AND number < 999999998",
            "max(number)\n999999997\n",
        ),
        (
            "SELECT sum(number) FROM numbers(1000000000) WHERE number = 999999999",
            "sum(number)\n999999999\n",
        ),
        (
            "SELECT sum(number) FROM numbers(1000000000) WHERE number > 1000000000",
            "sum(number)\nNULL\n",
        ),
        (
            "SELECT min(number) FROM numbers(0) WHERE number >= 0",
            "min(number)\nNULL\n",
        ),
    ] {
        assert_eq!(output(sql), expected, "{sql}");
    }
}

#[test]
fn stepped_and_non_interval_predicates_keep_reference_results() {
    for (sql, expected) in [
        (
            "SELECT sum(number) FROM generate_series(10, 22, 3) WHERE number >= 13 AND number < 22",
            "sum(number)\n48\n",
        ),
        (
            "SELECT max(number) FROM numbers(20) WHERE number != 19",
            "max(number)\n18\n",
        ),
        (
            "SELECT sum(number) FROM numbers(10) WHERE number % 2 = 0",
            "sum(number)\n20\n",
        ),
        (
            "SELECT min(number) FROM numbers(10) WHERE number = 11",
            "min(number)\nNULL\n",
        ),
        (
            "SELECT max(number) FROM numbers(10) WHERE number > 3 LIMIT 0",
            "max(number)\n",
        ),
    ] {
        assert_eq!(output(sql), expected, "{sql}");
    }
}
