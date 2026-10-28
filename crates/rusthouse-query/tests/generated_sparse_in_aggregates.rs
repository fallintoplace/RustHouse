use rusthouse_query::{execute, parse_select};
fn output(sql: &str) -> String {
    let mut result = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut result).unwrap();
    String::from_utf8(result).unwrap()
}

#[test]
fn sparse_in_set_aggregates_billion_rows_using_only_matching_positions() {
    for (sql, expected) in [
        ("SELECT sum(number) FROM numbers(1000000000) WHERE number IN (1, 9, 999999999, 9)", "sum(number)\n1000000009\n"),
        ("SELECT min(number) FROM numbers(1000000000) WHERE number IN (1, 9, 999999999)", "min(number)\n1\n"),
        ("SELECT max(number) FROM numbers(1000000000) WHERE number = 999999999", "max(number)\n999999999\n"),
        ("SELECT sum(number) FROM numbers(1000000000) WHERE number IN (1, 9, 999999999) AND number > 1", "sum(number)\n1000000008\n"),
        ("SELECT sum(number) FROM numbers(1000000000) WHERE number IN (1000000001, 1000000002)", "sum(number)\nNULL\n"),
        ("SELECT sum(number) FROM generate_series(10, 22, 3) WHERE number IN (10, 13, 22, 20)", "sum(number)\n45\n"),
    ] {
        assert_eq!(output(sql), expected, "{sql}");
    }
}
